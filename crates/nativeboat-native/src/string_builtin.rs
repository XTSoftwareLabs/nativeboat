//! The `String` constructor, its static functions and `String.prototype`,
//! as specified by ECMAScript 2024 (§22.1) and Annex B. Strings are UTF-16
//! code units; methods that work on code points keep unpaired surrogates.
//!
//! `localeCompare` collates with the system's ICU, as `Intl.Collator` does
//! in the user's locale by default. Without ICU it follows the root
//! collation in approximation: whitespace, punctuation and symbols sort before digits
//! and digits before letters; letters compare by their base letters, then
//! accents, then case (lowercase first), honoring the `numeric`,
//! `sensitivity`, `ignorePunctuation` and `caseFirst` options. The methods
//! taking regular expressions dispatch through `Symbol.match`,
//! `Symbol.replace`, `Symbol.search` and `Symbol.split`; string patterns are
//! handled here.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::array_builtin::{index_key, is_callable, to_integer_or_infinity, to_length};
use crate::object::{Key, Kind, Property, Wrapped};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::string::{JsStr, is_space};
use crate::symbol::Symbol;
use crate::unicode::{self, Form};
use crate::value::{Completion, Thrown, Value};
use core::cmp::Ordering;

/// The longest string engines create (V8's limit), beyond which operations
/// throw a `RangeError` instead of exhausting memory.
pub(crate) const MAX_STRING_LENGTH: usize = (1 << 29) - 24;

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn text(units: Vec<u16>) -> Value {
    Value::String(JsStr::from_units(&units))
}

/// Fails before allocating a string longer than engines allow.
fn bounded(realm: &Realm, length: f64) -> Result<(), Thrown> {
    if length > MAX_STRING_LENGTH as f64 {
        Err(realm.throw(ErrorKind::Range, "Invalid string length"))
    } else {
        Ok(())
    }
}

/// `RequireObjectCoercible(this)` then `ToString`.
fn this_text(realm: &mut Realm, this: &Value, method: &str) -> Result<JsStr, Thrown> {
    if this.is_nullish() {
        return Err(realm.throw(
            ErrorKind::Type,
            &format!("String.prototype.{method} called on null or undefined"),
        ));
    }
    ops::to_string(realm, this)
}

/// `thisStringValue`.
fn this_string_value(realm: &Realm, this: &Value, method: &str) -> Completion {
    match this {
        Value::String(_) => Ok(this.clone()),
        Value::Object(object) => match &object.data().kind {
            Kind::Primitive(Wrapped::String(text)) => Ok(Value::String(text.clone())),
            _ => Err(realm.throw(
                ErrorKind::Type,
                &format!("String.prototype.{method} requires a string"),
            )),
        },
        _ => Err(realm.throw(
            ErrorKind::Type,
            &format!("String.prototype.{method} requires a string"),
        )),
    }
}

/// `GetMethod`: `undefined` and `null` are absent; others must be callable.
fn get_method(realm: &mut Realm, value: &Value, key: &Key) -> Result<Option<Value>, Thrown> {
    let method = ops::get(realm, value, key)?;
    if method.is_nullish() {
        return Ok(None);
    }
    if !is_callable(&method) {
        return Err(realm.throw(
            ErrorKind::Type,
            &format!("{} is not a function", key.to_js()),
        ));
    }
    Ok(Some(method))
}

/// `StringIndexOf`.
fn index_of_units(text: &[u16], search: &[u16], from: usize) -> Option<usize> {
    if search.is_empty() {
        return (from <= text.len()).then_some(from);
    }
    if search.len() > text.len() {
        return None;
    }
    (from..=text.len() - search.len()).find(|&index| text[index..].starts_with(search))
}

fn position(realm: &mut Realm, value: &Value, length: usize) -> Result<usize, Thrown> {
    let position = to_integer_or_infinity(realm, value)?;
    Ok(position.clamp(0.0, length as f64) as usize)
}

fn at(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "at")?;
    let length = string.len() as f64;
    let relative = to_integer_or_infinity(realm, &arg(args, 0))?;
    let index = if relative >= 0.0 {
        relative
    } else {
        length + relative
    };
    if index < 0.0 || index >= length {
        return Ok(Value::Undefined);
    }
    Ok(text(vec![string.units()[index as usize]]))
}

/// The code unit at a position, if inside the string.
fn unit_at(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    method: &str,
) -> Result<(JsStr, Option<usize>), Thrown> {
    let string = this_text(realm, this, method)?;
    let position = to_integer_or_infinity(realm, &arg(args, 0))?;
    let inside = position >= 0.0 && position < string.len() as f64;
    Ok((string, inside.then_some(position as usize)))
}

fn char_at(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (string, index) = unit_at(realm, &this, args, "charAt")?;
    Ok(text(
        index
            .map(|index| vec![string.units()[index]])
            .unwrap_or_default(),
    ))
}

fn char_code_at(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (string, index) = unit_at(realm, &this, args, "charCodeAt")?;
    Ok(Value::Number(index.map_or(f64::NAN, |index| {
        f64::from(string.units()[index])
    })))
}

fn code_point_at(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (string, index) = unit_at(realm, &this, args, "codePointAt")?;
    let Some(index) = index else {
        return Ok(Value::Undefined);
    };
    let units = string.units();
    let point = unicode::code_points(&units[index..(index + 2).min(units.len())])[0];
    Ok(Value::Number(f64::from(point)))
}

fn concat(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let mut parts = vec![this_text(realm, &this, "concat")?];
    for value in args {
        parts.push(ops::to_string(realm, value)?);
    }
    bounded(realm, parts.iter().map(JsStr::len).sum::<usize>() as f64)?;
    Ok(text(
        parts
            .iter()
            .flat_map(|part| part.units().iter().copied())
            .collect(),
    ))
}

/// The receiver and a search string that must not be a regular expression.
fn searched(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    method: &str,
) -> Result<(JsStr, JsStr), Thrown> {
    let string = this_text(realm, this, method)?;
    let search = arg(args, 0);
    if crate::regexp_builtin::is_regexp(realm, &search)? {
        return Err(realm.throw(
            ErrorKind::Type,
            &format!(
                "First argument to String.prototype.{method} must not be a regular expression"
            ),
        ));
    }
    Ok((string, ops::to_string(realm, &search)?))
}

fn ends_with(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (string, search) = searched(realm, &this, args, "endsWith")?;
    let end = match arg(args, 1) {
        Value::Undefined => string.len(),
        value => position(realm, &value, string.len())?,
    };
    let units = &string.units()[..end];
    Ok(Value::Boolean(units.ends_with(search.units())))
}

fn starts_with(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (string, search) = searched(realm, &this, args, "startsWith")?;
    let start = position(realm, &arg(args, 1), string.len())?;
    Ok(Value::Boolean(
        string.units()[start..].starts_with(search.units()),
    ))
}

fn includes(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (string, search) = searched(realm, &this, args, "includes")?;
    let start = position(realm, &arg(args, 1), string.len())?;
    Ok(Value::Boolean(
        index_of_units(string.units(), search.units(), start).is_some(),
    ))
}

fn index_of(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "indexOf")?;
    let search = ops::to_string(realm, &arg(args, 0))?;
    let start = position(realm, &arg(args, 1), string.len())?;
    Ok(Value::Number(
        index_of_units(string.units(), search.units(), start).map_or(-1.0, |index| index as f64),
    ))
}

fn last_index_of(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "lastIndexOf")?;
    let search = ops::to_string(realm, &arg(args, 0))?;
    let number = ops::to_number(realm, &arg(args, 1))?;
    let start = if number.is_nan() {
        f64::INFINITY
    } else {
        crate::float::trunc(number)
    };
    let (units, search) = (string.units(), search.units());
    if search.len() > units.len() {
        return Ok(Value::Number(-1.0));
    }
    let last = start.clamp(0.0, (units.len() - search.len()) as f64) as usize;
    let found = (0..=last)
        .rev()
        .find(|&index| units[index..].starts_with(search));
    Ok(Value::Number(found.map_or(-1.0, |index| index as f64)))
}

fn is_well_formed(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let string = this_text(realm, &this, "isWellFormed")?;
    Ok(Value::Boolean(
        char::decode_utf16(string.units().iter().copied()).all(|unit| unit.is_ok()),
    ))
}

fn to_well_formed(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let string = this_text(realm, &this, "toWellFormed")?;
    let mut units = Vec::with_capacity(string.len());
    for decoded in char::decode_utf16(string.units().iter().copied()) {
        let character = decoded.unwrap_or(char::REPLACEMENT_CHARACTER);
        let mut buffer = [0; 2];
        units.extend_from_slice(character.encode_utf16(&mut buffer));
    }
    Ok(text(units))
}

fn normalize(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "normalize")?;
    let form = match arg(args, 0) {
        Value::Undefined => Form::Nfc,
        value => match ops::to_string(realm, &value)?.to_rust_lossy().as_str() {
            "NFC" => Form::Nfc,
            "NFD" => Form::Nfd,
            "NFKC" => Form::Nfkc,
            "NFKD" => Form::Nfkd,
            _ => {
                return Err(realm.throw(
                    ErrorKind::Range,
                    "The normalization form should be one of NFC, NFD, NFKC, NFKD.",
                ));
            }
        },
    };
    Ok(text(unicode::normalize(string.units(), form)))
}

/// `padStart` and `padEnd`.
fn pad(realm: &mut Realm, this: &Value, args: &[Value], method: &str, start: bool) -> Completion {
    let string = this_text(realm, this, method)?;
    let length = to_length(realm, &arg(args, 0))?;
    if length <= string.len() as u64 {
        return Ok(Value::String(string));
    }
    let filler = match arg(args, 1) {
        Value::Undefined => JsStr::from(" "),
        value => ops::to_string(realm, &value)?,
    };
    if filler.is_empty() {
        return Ok(Value::String(string));
    }
    bounded(realm, length as f64)?;
    let fill: Vec<u16> = filler
        .units()
        .iter()
        .copied()
        .cycle()
        .take(length as usize - string.len())
        .collect();
    let (mut first, second) = if start {
        (fill, string.units())
    } else {
        (string.units().to_vec(), &fill[..])
    };
    first.extend_from_slice(second);
    Ok(text(first))
}

fn repeat(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "repeat")?;
    let count = to_integer_or_infinity(realm, &arg(args, 0))?;
    if count < 0.0 || count == f64::INFINITY {
        return Err(realm.throw(ErrorKind::Range, "Invalid count value"));
    }
    if string.is_empty() || count == 0.0 {
        return Ok(text(Vec::new()));
    }
    bounded(realm, count * string.len() as f64)?;
    Ok(text(string.units().repeat(count as usize)))
}

fn slice(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "slice")?;
    let length = string.len() as f64;
    let bound = |realm: &mut Realm, value: &Value, fallback: f64| -> Result<usize, Thrown> {
        if matches!(value, Value::Undefined) {
            return Ok(fallback as usize);
        }
        let relative = to_integer_or_infinity(realm, value)?;
        Ok(if relative < 0.0 {
            (length + relative).max(0.0)
        } else {
            relative.min(length)
        } as usize)
    };
    let start = bound(realm, &arg(args, 0), 0.0)?;
    let end = bound(realm, &arg(args, 1), length)?;
    Ok(Value::String(string.slice(start, end.max(start))))
}

fn substring(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "substring")?;
    let start = position(realm, &arg(args, 0), string.len())?;
    let end = match arg(args, 1) {
        Value::Undefined => string.len(),
        value => position(realm, &value, string.len())?,
    };
    Ok(Value::String(string.slice(start.min(end), start.max(end))))
}

/// Annex B `substr`.
fn substr(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "substr")?;
    let size = string.len() as f64;
    let start = to_integer_or_infinity(realm, &arg(args, 0))?;
    let start = if start < 0.0 {
        (size + start).max(0.0)
    } else {
        start.min(size)
    };
    let length = match arg(args, 1) {
        Value::Undefined => size,
        value => to_integer_or_infinity(realm, &value)?,
    };
    let end = (start + length).min(size);
    if end <= start {
        return Ok(text(Vec::new()));
    }
    Ok(Value::String(string.slice(start as usize, end as usize)))
}

/// Whether a code point is a cased letter, for Final_Sigma.
fn cased(point: u32) -> bool {
    char::from_u32(point)
        .is_some_and(|character| character.is_lowercase() || character.is_uppercase())
}

/// The Final_Sigma condition at `index` of `points`: a cased letter before
/// and none after, skipping case-ignorable characters.
fn final_sigma(points: &[u32], index: usize) -> bool {
    let before = points[..index]
        .iter()
        .rev()
        .find(|&&point| !unicode::case_ignorable(point))
        .is_some_and(|&point| cased(point));
    let after = points[index + 1..]
        .iter()
        .find(|&&point| !unicode::case_ignorable(point))
        .is_some_and(|&point| cased(point));
    before && !after
}

/// Full Unicode case conversion of UTF-16 text.
pub(crate) fn convert_case(units: &[u16], upper: bool) -> Vec<u16> {
    let points = unicode::code_points(units);
    let mut converted = Vec::with_capacity(points.len());
    for (index, &point) in points.iter().enumerate() {
        match char::from_u32(point) {
            Some('\u{03A3}') if !upper && final_sigma(&points, index) => converted.push(0x03C2),
            Some(character) if upper => converted.extend(character.to_uppercase().map(u32::from)),
            Some(character) => converted.extend(character.to_lowercase().map(u32::from)),
            // Unpaired surrogates have no case.
            None => converted.push(point),
        }
    }
    unicode::units(&converted)
}

fn case(realm: &mut Realm, this: &Value, method: &str, upper: bool) -> Completion {
    let string = this_text(realm, this, method)?;
    Ok(text(convert_case(string.units(), upper)))
}

/// `trim`, `trimStart` and `trimEnd`.
fn trim(realm: &mut Realm, this: &Value, method: &str, start: bool, end: bool) -> Completion {
    let string = this_text(realm, this, method)?;
    let mut units = string.units();
    if start {
        let first = units
            .iter()
            .position(|&unit| !is_space(unit))
            .unwrap_or(units.len());
        units = &units[first..];
    }
    if end {
        let last = units
            .iter()
            .rposition(|&unit| !is_space(unit))
            .map_or(0, |index| index + 1);
        units = &units[..last];
    }
    Ok(text(units.to_vec()))
}

/// A collation element of `localeCompare`'s primary level.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Primary {
    Punctuation(u32),
    Digits(Vec<u32>),
    Letter(u32),
}

/// The collation of one string: primary elements, accents and cases.
struct Collation {
    primary: Vec<Primary>,
    accents: Vec<Vec<u32>>,
    cases: Vec<u8>,
}

struct CollationOptions {
    numeric: bool,
    accents: bool,
    case: bool,
    ignore_punctuation: bool,
    upper_first: bool,
}

fn fold(point: u32) -> u32 {
    char::from_u32(point)
        .and_then(|character| character.to_lowercase().next())
        .map_or(point, u32::from)
}

fn collate(units: &[u16], options: &CollationOptions) -> Collation {
    let points = unicode::code_points(&unicode::normalize(units, Form::Nfd));
    let mut collation = Collation {
        primary: Vec::new(),
        accents: Vec::new(),
        cases: Vec::new(),
    };
    let mut index = 0;
    while index < points.len() {
        let point = points[index];
        if options.numeric && (0x30..=0x39).contains(&point) {
            let end = points[index..]
                .iter()
                .position(|point| !(0x30..=0x39).contains(point))
                .map_or(points.len(), |offset| index + offset);
            let digits: Vec<u32> = points[index..end]
                .iter()
                .map(|digit| digit - 0x30)
                .collect();
            let significant = digits
                .iter()
                .position(|&digit| digit != 0)
                .unwrap_or(digits.len());
            let mut value = vec![(digits.len() - significant) as u32];
            value.extend_from_slice(&digits[significant..]);
            collation.primary.push(Primary::Digits(value));
            index = end;
            continue;
        }
        index += 1;
        if unicode::combining_class(point) != 0 {
            // A combining mark belongs to the preceding character's accents.
            match collation.accents.last_mut() {
                Some(accents) => accents.push(point),
                None => collation.accents.push(vec![point]),
            }
            continue;
        }
        if unicode::punctuation(point) {
            if !options.ignore_punctuation {
                collation.primary.push(Primary::Punctuation(point));
            }
            continue;
        }
        // Compatibility variants (full-width forms, ligatures) share their
        // plain characters' primary weights and differ at the third level.
        let compatible =
            unicode::code_points(&unicode::normalize(&unicode::units(&[point]), Form::Nfkd));
        let mut accents = Vec::new();
        for &part in &compatible {
            if unicode::combining_class(part) != 0 {
                accents.push(part);
            } else if (0x30..=0x39).contains(&part) {
                collation
                    .primary
                    .push(Primary::Digits(vec![1, part - 0x30]));
            } else {
                collation.primary.push(Primary::Letter(fold(part)));
            }
        }
        collation.accents.push(accents);
        let upper = char::from_u32(point).is_some_and(char::is_uppercase);
        let variant = compatible != [point];
        collation
            .cases
            .push(u8::from(upper != options.upper_first) * 2 + u8::from(variant));
    }
    collation
}

fn order(ordering: Ordering) -> Value {
    Value::Number(match ordering {
        Ordering::Less => -1.0,
        Ordering::Equal => 0.0,
        Ordering::Greater => 1.0,
    })
}

fn collation_options(realm: &mut Realm, options: &Value) -> Result<CollationOptions, Thrown> {
    let mut result = CollationOptions {
        numeric: false,
        accents: true,
        case: true,
        ignore_punctuation: false,
        upper_first: false,
    };
    if matches!(options, Value::Undefined) {
        return Ok(result);
    }
    let options = Value::Object(ops::to_object(realm, options)?);
    result.numeric = ops::to_boolean(&ops::get_named(realm, &options, "numeric")?);
    result.ignore_punctuation =
        ops::to_boolean(&ops::get_named(realm, &options, "ignorePunctuation")?);
    let sensitivity = ops::get_named(realm, &options, "sensitivity")?;
    if !matches!(sensitivity, Value::Undefined) {
        let (accents, case) = match ops::to_string(realm, &sensitivity)?
            .to_rust_lossy()
            .as_str()
        {
            "base" => (false, false),
            "accent" => (true, false),
            "case" => (false, true),
            "variant" => (true, true),
            _ => return Err(realm.throw(ErrorKind::Range, "Value out of range for sensitivity")),
        };
        (result.accents, result.case) = (accents, case);
    }
    let case_first = ops::get_named(realm, &options, "caseFirst")?;
    if !matches!(case_first, Value::Undefined) {
        result.upper_first = match ops::to_string(realm, &case_first)?.to_rust_lossy().as_str() {
            "upper" => true,
            "lower" | "false" => false,
            _ => return Err(realm.throw(ErrorKind::Range, "Value out of range for caseFirst")),
        };
    }
    Ok(result)
}

fn locale_compare(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let string = this_text(realm, &this, "localeCompare")?;
    let that = ops::to_string(realm, &arg(args, 0))?;
    if let Some(order) = crate::intl_builtin::locale_compare(
        realm,
        string.units(),
        that.units(),
        &arg(args, 1),
        &arg(args, 2),
    )? {
        return Ok(Value::Number(f64::from(order.signum())));
    }
    root_compare(realm, &string, &that, &arg(args, 2))
}

/// The approximate root collation used without ICU.
fn root_compare(realm: &mut Realm, string: &JsStr, that: &JsStr, options: &Value) -> Completion {
    let options = collation_options(realm, options)?;
    let (left, right) = (
        collate(string.units(), &options),
        collate(that.units(), &options),
    );
    let mut ordering = left.primary.cmp(&right.primary);
    if ordering == Ordering::Equal && options.accents {
        ordering = left.accents.cmp(&right.accents);
    }
    if ordering == Ordering::Equal && options.case {
        ordering = left.cases.cmp(&right.cases);
    }
    // Like ICU at its default (tertiary) strength, strings equal at these
    // three levels compare equal.
    Ok(order(ordering))
}

/// The replacement for a string match: the replacer's result or the
/// substituted template.
fn replacement(
    realm: &mut Realm,
    replacer: &Value,
    matched: &JsStr,
    string: &JsStr,
    position: usize,
) -> Result<Vec<u16>, Thrown> {
    if is_callable(replacer) {
        let result = ops::call(
            realm,
            replacer,
            Value::Undefined,
            &[
                Value::String(matched.clone()),
                Value::Number(position as f64),
                Value::String(string.clone()),
            ],
        )?;
        Ok(ops::to_string(realm, &result)?.units().to_vec())
    } else {
        let template = ops::to_string(realm, replacer)?;
        crate::regexp_builtin::substitution(
            realm,
            matched.units(),
            string.units(),
            position,
            &[],
            &Value::Undefined,
            template.units(),
        )
    }
}

/// Dispatches to the argument's `Symbol` method, as the regular expression
/// methods do.
fn delegate(
    realm: &mut Realm,
    value: &Value,
    symbol: Symbol,
    args: &[Value],
) -> Result<Option<Value>, Thrown> {
    if value.is_nullish() {
        return Ok(None);
    }
    match get_method(realm, value, &Key::Symbol(symbol))? {
        Some(method) => Ok(Some(ops::call(realm, &method, value.clone(), args)?)),
        None => Ok(None),
    }
}

fn replace(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (search, replacer) = (arg(args, 0), arg(args, 1));
    if this.is_nullish() {
        return this_text(realm, &this, "replace").map(Value::String);
    }
    let symbol = realm.symbols.replace.clone();
    if let Some(result) = delegate(realm, &search, symbol, &[this.clone(), replacer.clone()])? {
        return Ok(result);
    }
    let string = ops::to_string(realm, &this)?;
    let search = ops::to_string(realm, &search)?;
    let replacer = if is_callable(&replacer) {
        replacer
    } else {
        Value::String(ops::to_string(realm, &replacer)?)
    };
    let Some(position) = index_of_units(string.units(), search.units(), 0) else {
        return Ok(Value::String(string));
    };
    let mut result = string.units()[..position].to_vec();
    result.extend(replacement(realm, &replacer, &search, &string, position)?);
    result.extend_from_slice(&string.units()[position + search.len()..]);
    Ok(text(result))
}

fn replace_all(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (search, replacer) = (arg(args, 0), arg(args, 1));
    if this.is_nullish() {
        return this_text(realm, &this, "replaceAll").map(Value::String);
    }
    if crate::regexp_builtin::is_regexp(realm, &search)? {
        let flags = ops::get_named(realm, &search, "flags")?;
        if flags.is_nullish() {
            return Err(realm.throw(
                ErrorKind::Type,
                "The regular expression flags are null or undefined",
            ));
        }
        let flags = ops::to_string(realm, &flags)?;
        if !flags.units().contains(&u16::from(b'g')) {
            return Err(realm.throw(
                ErrorKind::Type,
                "replaceAll must be called with a global RegExp",
            ));
        }
    }
    let symbol = realm.symbols.replace.clone();
    if let Some(result) = delegate(realm, &search, symbol, &[this.clone(), replacer.clone()])? {
        return Ok(result);
    }
    let string = ops::to_string(realm, &this)?;
    let search = ops::to_string(realm, &search)?;
    let replacer = if is_callable(&replacer) {
        replacer
    } else {
        Value::String(ops::to_string(realm, &replacer)?)
    };
    let advance = search.len().max(1);
    let mut positions = Vec::new();
    let mut from = index_of_units(string.units(), search.units(), 0);
    while let Some(position) = from {
        positions.push(position);
        from = index_of_units(string.units(), search.units(), position + advance);
    }
    let mut result = Vec::new();
    let mut end = 0;
    for position in positions {
        result.extend_from_slice(&string.units()[end..position]);
        result.extend(replacement(realm, &replacer, &search, &string, position)?);
        end = position + search.len();
    }
    result.extend_from_slice(&string.units()[end..]);
    Ok(text(result))
}

/// `match`, `matchAll` and `search`: the argument's method, or a new
/// regular expression's.
fn regexp_method(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    method: &str,
    symbol: Symbol,
    flags: &str,
) -> Completion {
    let pattern = arg(args, 0);
    if this.is_nullish() {
        return this_text(realm, this, method).map(Value::String);
    }
    if let Some(result) = delegate(realm, &pattern, symbol.clone(), core::slice::from_ref(this))? {
        return Ok(result);
    }
    let string = Value::String(ops::to_string(realm, this)?);
    let regexp = crate::regexp_builtin::create(realm, &pattern, flags)?;
    let method = ops::get(realm, &regexp, &Key::Symbol(symbol))?;
    ops::call(realm, &method, regexp, &[string])
}

fn match_all(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let pattern = arg(args, 0);
    if !this.is_nullish() && crate::regexp_builtin::is_regexp(realm, &pattern)? {
        let flags = ops::get_named(realm, &pattern, "flags")?;
        if flags.is_nullish() {
            return Err(realm.throw(
                ErrorKind::Type,
                "The regular expression flags are null or undefined",
            ));
        }
        let flags = ops::to_string(realm, &flags)?;
        if !flags.units().contains(&u16::from(b'g')) {
            return Err(realm.throw(
                ErrorKind::Type,
                "matchAll must be called with a global RegExp",
            ));
        }
    }
    let symbol = realm.symbols.match_all.clone();
    regexp_method(realm, &this, args, "matchAll", symbol, "g")
}

fn split(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (separator, limit) = (arg(args, 0), arg(args, 1));
    if this.is_nullish() {
        return this_text(realm, &this, "split").map(Value::String);
    }
    let symbol = realm.symbols.split.clone();
    if let Some(result) = delegate(realm, &separator, symbol, &[this.clone(), limit.clone()])? {
        return Ok(result);
    }
    let string = ops::to_string(realm, &this)?;
    let limit = match limit {
        Value::Undefined => u32::MAX,
        value => ops::to_uint32(realm, &value)?,
    } as usize;
    let separator_text = ops::to_string(realm, &separator)?;
    let mut parts: Vec<Value> = Vec::new();
    if limit == 0 {
        return Ok(Value::Object(realm.array(parts)));
    }
    if matches!(separator, Value::Undefined) {
        return Ok(Value::Object(realm.array(vec![Value::String(string)])));
    }
    let (units, search) = (string.units(), separator_text.units());
    if search.is_empty() {
        parts = units
            .iter()
            .take(limit)
            .map(|&unit| text(vec![unit]))
            .collect();
        return Ok(Value::Object(realm.array(parts)));
    }
    let mut start = 0;
    while let Some(found) = index_of_units(units, search, start) {
        parts.push(text(units[start..found].to_vec()));
        if parts.len() == limit {
            return Ok(Value::Object(realm.array(parts)));
        }
        start = found + search.len();
    }
    parts.push(text(units[start..].to_vec()));
    Ok(Value::Object(realm.array(parts)))
}

/// Annex B HTML methods: `<tag name="value">string</tag>`.
fn html(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    method: &str,
    tag: &str,
    attribute: Option<&str>,
) -> Completion {
    let string = this_text(realm, this, method)?;
    let mut result = format!("<{tag}");
    if let Some(attribute) = attribute {
        let value = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        result.push_str(&format!(
            " {attribute}=\"{}\"",
            value.replace('"', "&quot;")
        ));
    }
    result.push('>');
    let mut units: Vec<u16> = result.encode_utf16().collect();
    units.extend_from_slice(string.units());
    units.extend(format!("</{tag}>").encode_utf16());
    Ok(text(units))
}

fn from_char_code(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let mut units = Vec::with_capacity(args.len());
    for value in args {
        units.push(ops::to_uint32(realm, value)? as u16);
    }
    Ok(text(units))
}

fn from_code_point(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let mut points = Vec::with_capacity(args.len());
    for value in args {
        let number = ops::to_number(realm, value)?;
        if !(0.0..=1_114_111.0).contains(&number) || crate::float::fract(number) != 0.0 {
            let shown = ops::to_string(realm, &Value::Number(number))?;
            return Err(realm.throw(ErrorKind::Range, &format!("Invalid code point {shown}")));
        }
        points.push(number as u32);
    }
    Ok(text(unicode::units(&points)))
}

fn raw(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let cooked = Value::Object(ops::to_object(realm, &arg(args, 0))?);
    let literals = ops::get_named(realm, &cooked, "raw")?;
    let literals = Value::Object(ops::to_object(realm, &literals)?);
    let count = crate::array_builtin::length_of(realm, &literals)?;
    let mut result = Vec::new();
    for index in 0..count {
        let segment = ops::get(realm, &literals, &index_key(index))?;
        result.extend_from_slice(ops::to_string(realm, &segment)?.units());
        if index + 1 == count {
            break;
        }
        if let Some(substitution) = args.get(index as usize + 1) {
            result.extend_from_slice(ops::to_string(realm, substitution)?.units());
        }
    }
    Ok(text(result))
}

/// `String(value)` and `new String(value)`.
fn string_value(realm: &mut Realm, args: &[Value], called: bool) -> Result<JsStr, Thrown> {
    match args.first() {
        None => Ok(JsStr::default()),
        Some(Value::Symbol(symbol)) if called => Ok(symbol.descriptive()),
        Some(value) => ops::to_string(realm, value),
    }
}

/// Installs `String` and `String.prototype`'s methods.
pub(crate) fn install(realm: &mut Realm) {
    let prototype = realm.string_prototype.clone();
    // `String.prototype` is itself a String object holding the empty string.
    prototype.data_mut().kind = Kind::Primitive(Wrapped::String(JsStr::default()));
    let constructor = realm.builtin_constructor(
        "String",
        1,
        &prototype,
        |realm, _, args| Ok(Value::String(string_value(realm, args, true)?)),
        |realm, target, args| {
            let value = string_value(realm, args, false)?;
            let object = realm.primitive_object(Wrapped::String(value));
            let prototype = ops::get_named(realm, &Value::Object(target), "prototype")?;
            if let Value::Object(prototype) = prototype {
                object.set_prototype(Some(prototype));
            }
            Ok(object)
        },
    );
    realm.method(&constructor, "fromCharCode", 1, from_char_code);
    realm.method(&constructor, "fromCodePoint", 1, from_code_point);
    realm.method(&constructor, "raw", 1, raw);
    realm.global.define(
        Key::name("String"),
        Property::hidden(Value::Object(constructor)),
    );
    type Method = fn(&mut Realm, Value, &[Value]) -> Completion;
    let methods: [(&str, u32, Method); 33] = [
        ("at", 1, at),
        ("charAt", 1, char_at),
        ("charCodeAt", 1, char_code_at),
        ("codePointAt", 1, code_point_at),
        ("concat", 1, concat),
        ("endsWith", 1, ends_with),
        ("includes", 1, includes),
        ("indexOf", 1, index_of),
        ("isWellFormed", 0, is_well_formed),
        ("lastIndexOf", 1, last_index_of),
        ("localeCompare", 1, locale_compare),
        ("match", 1, |realm, this, args| {
            let symbol = realm.symbols.match_.clone();
            regexp_method(realm, &this, args, "match", symbol, "")
        }),
        ("matchAll", 1, match_all),
        ("normalize", 0, normalize),
        ("padEnd", 1, |realm, this, args| {
            pad(realm, &this, args, "padEnd", false)
        }),
        ("padStart", 1, |realm, this, args| {
            pad(realm, &this, args, "padStart", true)
        }),
        ("repeat", 1, repeat),
        ("replace", 2, replace),
        ("replaceAll", 2, replace_all),
        ("search", 1, |realm, this, args| {
            let symbol = realm.symbols.search.clone();
            regexp_method(realm, &this, args, "search", symbol, "")
        }),
        ("slice", 2, slice),
        ("split", 2, split),
        ("startsWith", 1, starts_with),
        ("substr", 2, substr),
        ("substring", 2, substring),
        ("toLocaleLowerCase", 0, |realm, this, _| {
            case(realm, &this, "toLocaleLowerCase", false)
        }),
        ("toLocaleUpperCase", 0, |realm, this, _| {
            case(realm, &this, "toLocaleUpperCase", true)
        }),
        ("toLowerCase", 0, |realm, this, _| {
            case(realm, &this, "toLowerCase", false)
        }),
        ("toString", 0, |realm, this, _| {
            this_string_value(realm, &this, "toString")
        }),
        ("toUpperCase", 0, |realm, this, _| {
            case(realm, &this, "toUpperCase", true)
        }),
        ("toWellFormed", 0, to_well_formed),
        ("trim", 0, |realm, this, _| {
            trim(realm, &this, "trim", true, true)
        }),
        ("valueOf", 0, |realm, this, _| {
            this_string_value(realm, &this, "valueOf")
        }),
    ];
    for (name, length, code) in methods {
        realm.method(&prototype, name, length, code);
    }
    // Annex B: trimLeft and trimRight are the same functions.
    let trim_start = realm.function("trimStart", 0, false, |realm, this, _| {
        trim(realm, &this, "trimStart", true, false)
    });
    let trim_end = realm.function("trimEnd", 0, false, |realm, this, _| {
        trim(realm, &this, "trimEnd", false, true)
    });
    for (names, function) in [
        (["trimStart", "trimLeft"], trim_start),
        (["trimEnd", "trimRight"], trim_end),
    ] {
        for name in names {
            prototype.define(
                Key::name(name),
                Property::hidden(Value::Object(function.clone())),
            );
        }
    }
    type Html = (&'static str, &'static str, Option<&'static str>);
    let tags: [Html; 13] = [
        ("anchor", "a", Some("name")),
        ("big", "big", None),
        ("blink", "blink", None),
        ("bold", "b", None),
        ("fixed", "tt", None),
        ("fontcolor", "font", Some("color")),
        ("fontsize", "font", Some("size")),
        ("italics", "i", None),
        ("link", "a", Some("href")),
        ("small", "small", None),
        ("strike", "strike", None),
        ("sub", "sub", None),
        ("sup", "sup", None),
    ];
    for (name, tag, attribute) in tags {
        let length = u32::from(attribute.is_some());
        realm.method(&prototype, name, length, move |realm, this, args| {
            html(realm, &this, args, name, tag, attribute)
        });
    }
}

#[cfg(test)]
mod tests;
