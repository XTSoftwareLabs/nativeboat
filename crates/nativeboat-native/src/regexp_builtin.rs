//! `RegExp` objects over the native UTF-16 matcher, as ECMAScript 2024
//! specifies them and Node 22 behaves: the constructor and literal
//! creation, `exec`/`test`/`toString`, flag and `source` accessors, the
//! `Symbol.match`/`matchAll`/`replace`/`search`/`split` protocols with
//! `lastIndex` and species semantics, and the RegExp String Iterator.
//! Annex B's `compile` and legacy static properties (`RegExp.$1`) are not
//! provided.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::array_builtin::{is_callable, is_constructor, length_of, to_length};
use crate::object::{Key, Kind, Object, Property, Slot, WeakObject};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::string::JsStr;
use crate::value::{Completion, Thrown, Value};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use nativeboat_regexp::{Flags, Match, Pattern};

/// The internal slots of a regular expression object.
#[derive(Clone, Debug)]
pub struct RegExpData {
    /// `[[OriginalSource]]`.
    pub source: JsStr,
    /// `[[OriginalFlags]]`.
    pub flags: JsStr,
    pattern: Rc<Pattern>,
}

impl RegExpData {
    fn has(&self, flag: u8) -> bool {
        self.flags.units().contains(&u16::from(flag))
    }
}

/// The state of a RegExp String Iterator.
#[derive(Debug)]
pub struct StringIterator {
    matcher: Value,
    string: JsStr,
    global: bool,
    unicode: bool,
    done: bool,
}

/// Where a literal's source and flags are: their addresses and length.
type LiteralSite = (usize, usize, usize);

/// A literal's source and flags as strings, and its pattern.
type CompiledLiteral = (JsStr, JsStr, Rc<Pattern>);

/// Compiled patterns by source and flags. Literals evaluated repeatedly and
/// repeated constructions share one compilation.
#[derive(Default)]
pub(crate) struct Patterns {
    compiled: crate::table::HashMap<(Vec<u16>, Vec<u16>), Rc<Pattern>>,
    count: usize,
    /// Literals by the address of their source and flags in the program:
    /// their strings and pattern, so evaluating one copies nothing.
    literals: crate::table::HashMap<LiteralSite, CompiledLiteral>,
    literal_count: usize,
}

impl core::fmt::Debug for Patterns {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "Patterns({})", self.count)
    }
}

/// The cache is emptied when it reaches this many patterns, bounding the
/// memory that dynamically built patterns can hold.
const CACHE_LIMIT: usize = 512;

const FLAGS: &[u8] = b"dgimsuvy";

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn has_unit(text: &JsStr, flag: u8) -> bool {
    text.units().contains(&u16::from(flag))
}

/// Validates flags: only `dgimsuvy`, each at most once, not both `u` and `v`.
fn matcher_flags(flags: &JsStr) -> Option<Flags> {
    let units = flags.units();
    for (index, unit) in units.iter().enumerate() {
        let valid = u8::try_from(*unit).is_ok_and(|unit| FLAGS.contains(&unit));
        if !valid || units[..index].contains(unit) {
            return None;
        }
    }
    let has = |flag| has_unit(flags, flag);
    if has(b'u') && has(b'v') {
        return None;
    }
    Some(Flags {
        ignore_case: has(b'i'),
        multiline: has(b'm'),
        dot_all: has(b's'),
        unicode: has(b'u'),
        unicode_sets: has(b'v'),
    })
}

/// `RegExpInitialize`'s parsing: a compiled pattern, or the SyntaxError
/// message for invalid flags or syntax.
pub(crate) fn compile(
    realm: &mut Realm,
    source: &JsStr,
    flags: &JsStr,
) -> Result<Rc<Pattern>, String> {
    let Some(options) = matcher_flags(flags) else {
        return Err(format!(
            "Invalid flags supplied to RegExp constructor '{}'",
            flags.to_rust_lossy()
        ));
    };
    let key = (source.units().to_vec(), flags.units().to_vec());
    if let Some(pattern) = realm.patterns.compiled.get(&key) {
        return Ok(pattern.clone());
    }
    let pattern = Pattern::compile(source.units(), options).map_err(|message| {
        format!(
            "Invalid regular expression: /{}/{}: {message}",
            source.to_rust_lossy(),
            flags.to_rust_lossy()
        )
    })?;
    let pattern = Rc::new(pattern);
    if realm.patterns.count == CACHE_LIMIT {
        realm.patterns = Patterns::default();
    }
    realm.patterns.count += 1;
    realm.patterns.compiled.insert(key, pattern.clone());
    Ok(pattern)
}

/// The regular expression data of a value, if it is a RegExp object.
fn data_of(value: &Value) -> Option<RegExpData> {
    match &value.as_object()?.data().kind {
        Kind::RegExp(data) => Some(data.clone()),
        _ => None,
    }
}

fn last_index_key() -> Key {
    crate::string::static_key!("lastIndex")
}

/// `RegExpAlloc` and `RegExpInitialize`: a new regular expression object.
fn allocate(
    realm: &mut Realm,
    prototype: Object,
    source: JsStr,
    flags: JsStr,
) -> Result<Object, Thrown> {
    let pattern = compile(realm, &source, &flags)
        .map_err(|message| realm.throw(ErrorKind::Syntax, &message))?;
    Ok(regexp_object(prototype, source, flags, pattern))
}

/// A new regular expression object of a compiled pattern.
fn regexp_object(prototype: Object, source: JsStr, flags: JsStr, pattern: Rc<Pattern>) -> Object {
    let object = Object::new(
        Some(prototype),
        Kind::RegExp(RegExpData {
            source,
            flags,
            pattern,
        }),
    );
    object.define(
        last_index_key(),
        Property {
            slot: Slot::Data {
                value: Value::Number(0.0),
                writable: true,
            },
            enumerable: false,
            configurable: false,
        },
    );
    object
}

/// A regular expression literal: a new object for each evaluation, sharing
/// the pattern compiled for its source and flags.
pub fn literal(realm: &mut Realm, source: &[u16], flags: &str) -> Completion {
    let prototype = realm.regexp_prototype.clone();
    let site = (
        source.as_ptr() as usize,
        source.len(),
        flags.as_ptr() as usize,
    );
    // Callers other than compiled code may pass buffers whose addresses are
    // reused, so a known site still checks its text.
    let known = realm
        .patterns
        .literals
        .get(&site)
        .filter(|(text, flag_text, _)| {
            text.units() == source && flag_text.units().iter().copied().eq(flags.encode_utf16())
        });
    if let Some((source, flags, pattern)) = known.cloned() {
        return Ok(Value::Object(regexp_object(
            prototype, source, flags, pattern,
        )));
    }
    let (source, flags) = (JsStr::from_units(source), JsStr::from(flags));
    let object = allocate(realm, prototype, source.clone(), flags.clone())?;
    if let Some(data) = data_of(&Value::Object(object.clone())) {
        let patterns = &mut realm.patterns;
        if patterns.literal_count == CACHE_LIMIT {
            patterns.literals = Default::default();
            patterns.literal_count = 0;
        }
        patterns.literal_count += 1;
        patterns
            .literals
            .insert(site, (source, flags, data.pattern));
    }
    Ok(Value::Object(object))
}

/// `RegExpCreate`: a regular expression from the intrinsic constructor, as
/// String methods create one for a non-RegExp argument.
pub(crate) fn create(realm: &mut Realm, pattern: &Value, flags: &str) -> Completion {
    let source = match pattern {
        Value::Undefined => JsStr::default(),
        pattern => ops::to_string(realm, pattern)?,
    };
    let prototype = realm.regexp_prototype.clone();
    Ok(Value::Object(allocate(
        realm,
        prototype,
        source,
        JsStr::from(flags),
    )?))
}

/// The RegExp constructor's shared behavior for calls (`target` absent) and
/// construction.
fn construct(
    realm: &mut Realm,
    constructor: &Object,
    target: Option<Object>,
    args: &[Value],
) -> Result<Object, Thrown> {
    let (pattern, flags) = (arg(args, 0), arg(args, 1));
    let pattern_is_regexp = is_regexp(realm, &pattern)?;
    let target = match target {
        Some(target) => target,
        None => {
            if pattern_is_regexp && matches!(flags, Value::Undefined) {
                let pattern_constructor = ops::get_named(realm, &pattern, "constructor")?;
                if matches!(&pattern_constructor, Value::Object(object) if object == constructor) {
                    return Ok(pattern
                        .as_object()
                        .expect("regular expressions are objects")
                        .clone());
                }
            }
            constructor.clone()
        }
    };
    let (source, flags) = if let Some(data) = data_of(&pattern) {
        let flags = match flags {
            Value::Undefined => Value::String(data.flags),
            flags => flags,
        };
        (Value::String(data.source), flags)
    } else if pattern_is_regexp {
        let source = ops::get_named(realm, &pattern, "source")?;
        let flags = match flags {
            Value::Undefined => ops::get_named(realm, &pattern, "flags")?,
            flags => flags,
        };
        (source, flags)
    } else {
        (pattern, flags)
    };
    let fallback = realm.regexp_prototype.clone();
    let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
        Value::Object(prototype) => prototype,
        _ => fallback,
    };
    let text = |realm: &mut Realm, value: &Value| match value {
        Value::Undefined => Ok(JsStr::default()),
        value => ops::to_string(realm, value),
    };
    let source = text(realm, &source)?;
    let flags = text(realm, &flags)?;
    allocate(realm, prototype, source, flags)
}

/// `IsRegExp`.
pub(crate) fn is_regexp(realm: &mut Realm, value: &Value) -> Result<bool, Thrown> {
    if !matches!(value, Value::Object(_)) {
        return Ok(false);
    }
    let key = Key::Symbol(realm.symbols.match_.clone());
    match ops::get(realm, value, &key)? {
        Value::Undefined => Ok(data_of(value).is_some()),
        flag => Ok(ops::to_boolean(&flag)),
    }
}

/// `AdvanceStringIndex`.
fn advance(text: &[u16], index: u64, unicode: bool) -> u64 {
    let next = index + 1;
    if !unicode || next >= text.len() as u64 {
        return next;
    }
    let lead = text[index as usize];
    let trail = text[next as usize];
    if (0xD800..=0xDBFF).contains(&lead) && (0xDC00..=0xDFFF).contains(&trail) {
        next + 1
    } else {
        next
    }
}

fn set_last_index(realm: &mut Realm, regexp: &Value, index: u64) -> Result<(), Thrown> {
    ops::set(
        realm,
        regexp,
        &last_index_key(),
        Value::Number(index as f64),
    )
}

/// The index and match array of a native match.
fn match_array(realm: &Realm, data: &RegExpData, string: &JsStr, matched: &Match) -> Value {
    let substring =
        |range: &core::ops::Range<usize>| Value::String(string.slice(range.start, range.end));
    // The match and its captures, then index, input, groups and indices.
    // These are new objects, so defining their properties cannot fail.
    let mut elements = vec![substring(&matched.range)];
    for capture in &matched.captures {
        elements.push(capture.as_ref().map(substring).unwrap_or_default());
    }
    let array = realm.array(elements);
    let put = |object: &Object, key: Key, value: Value| {
        object.define(key, Property::data(value));
    };
    put(
        &array,
        crate::string::static_key!("index"),
        Value::Number(matched.range.start as f64),
    );
    put(
        &array,
        crate::string::static_key!("input"),
        Value::String(string.clone()),
    );
    let named = |values: &mut dyn Iterator<Item = (&String, Value)>| {
        let groups = Object::new(None, Kind::Ordinary);
        let mut any = false;
        for (name, value) in values {
            put(&groups, Key::name(name), value);
            any = true;
        }
        if any {
            Value::Object(groups)
        } else {
            Value::Undefined
        }
    };
    let mut groups = matched
        .groups
        .iter()
        .map(|(name, range)| (name, range.as_ref().map(substring).unwrap_or_default()));
    put(
        &array,
        crate::string::static_key!("groups"),
        named(&mut groups),
    );
    if data.has(b'd') {
        let pair = |range: &Option<core::ops::Range<usize>>| match range {
            Some(range) => {
                let pair = vec![
                    Value::Number(range.start as f64),
                    Value::Number(range.end as f64),
                ];
                Value::Object(realm.array(pair))
            }
            None => Value::Undefined,
        };
        let mut pairs = vec![pair(&Some(matched.range.clone()))];
        pairs.extend(matched.captures.iter().map(pair));
        let indices = realm.array(pairs.clone());
        let mut groups = matched
            .group_indices
            .iter()
            .map(|(name, index)| (name, pairs[*index].clone()));
        put(
            &indices,
            crate::string::static_key!("groups"),
            named(&mut groups),
        );
        put(
            &array,
            crate::string::static_key!("indices"),
            Value::Object(indices),
        );
    }
    Value::Object(array)
}

/// `RegExpBuiltinExec`.
fn builtin_exec(
    realm: &mut Realm,
    regexp: &Value,
    data: &RegExpData,
    string: &JsStr,
) -> Completion {
    let last_index = ops::get(realm, regexp, &last_index_key())?;
    let mut last_index = to_length(realm, &last_index)?;
    let (global, sticky) = (data.has(b'g'), data.has(b'y'));
    if !global && !sticky {
        last_index = 0;
    }
    let found = if last_index > string.len() as u64 {
        None
    } else {
        data.pattern
            .search(string.units(), last_index as usize)
            .filter(|matched| !sticky || matched.range.start as u64 <= last_index)
    };
    let Some(matched) = found else {
        if global || sticky {
            set_last_index(realm, regexp, 0)?;
        }
        return Ok(Value::Null);
    };
    if global || sticky {
        set_last_index(realm, regexp, matched.range.end as u64)?;
    }
    Ok(match_array(realm, data, string, &matched))
}

/// `RegExpExec`: a user `exec` method, or the built-in one.
fn exec(realm: &mut Realm, regexp: &Value, string: &JsStr) -> Completion {
    let method = ops::get_named(realm, regexp, "exec")?;
    if is_callable(&method) {
        let result = ops::call(
            realm,
            &method,
            regexp.clone(),
            &[Value::String(string.clone())],
        )?;
        if !matches!(result, Value::Object(_) | Value::Null) {
            return Err(realm.throw(
                ErrorKind::Type,
                "RegExp exec method returned something other than an Object or null",
            ));
        }
        return Ok(result);
    }
    let Some(data) = data_of(regexp) else {
        return Err(realm.throw(
            ErrorKind::Type,
            "RegExp exec method called on an incompatible receiver",
        ));
    };
    builtin_exec(realm, regexp, &data, string)
}

/// The receiver of a `Symbol` protocol method, which must be an object.
fn receiver(realm: &Realm, this: &Value, method: &str) -> Result<(), Thrown> {
    match this {
        Value::Object(_) => Ok(()),
        // V8 names the receiver, which tells where a detached method went.
        _ => {
            let shown = match this {
                Value::Undefined => String::from("undefined"),
                Value::Null => String::from("null"),
                Value::Boolean(value) => format!("{value}"),
                Value::Number(value) => format!("{value}"),
                Value::String(text) => text.to_rust_lossy(),
                Value::Symbol(_) => String::from("Symbol()"),
                _ => String::new(),
            };
            Err(realm.throw(
                ErrorKind::Type,
                &format!(
                    "Method RegExp.prototype.{method} called on incompatible receiver {shown}"
                ),
            ))
        }
    }
}

fn flags_of(realm: &mut Realm, regexp: &Value) -> Result<JsStr, Thrown> {
    let flags = ops::get_named(realm, regexp, "flags")?;
    ops::to_string(realm, &flags)
}

fn unicode_flags(flags: &JsStr) -> bool {
    has_unit(flags, b'u') || has_unit(flags, b'v')
}

/// After an empty match, moves `lastIndex` past it so a global search ends.
fn step_past_empty(
    realm: &mut Realm,
    regexp: &Value,
    result: &Value,
    string: &JsStr,
    unicode: bool,
) -> Result<JsStr, Thrown> {
    let matched = ops::get(realm, result, &Key::Index(0))?;
    let matched = ops::to_string(realm, &matched)?;
    if matched.is_empty() {
        let index = ops::get(realm, regexp, &last_index_key())?;
        let index = to_length(realm, &index)?;
        set_last_index(realm, regexp, advance(string.units(), index, unicode))?;
    }
    Ok(matched)
}

fn symbol_match(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    receiver(realm, &this, "@@match")?;
    let string = ops::to_string(realm, &arg(args, 0))?;
    let flags = flags_of(realm, &this)?;
    if !has_unit(&flags, b'g') {
        return exec(realm, &this, &string);
    }
    let unicode = unicode_flags(&flags);
    set_last_index(realm, &this, 0)?;
    let mut matches = Vec::new();
    loop {
        let result = exec(realm, &this, &string)?;
        if matches!(result, Value::Null) {
            return Ok(if matches.is_empty() {
                Value::Null
            } else {
                Value::Object(realm.array(matches))
            });
        }
        let matched = step_past_empty(realm, &this, &result, &string, unicode)?;
        matches.push(Value::String(matched));
    }
}

/// `SpeciesConstructor`.
fn species_constructor(realm: &mut Realm, object: &Value, fallback: &Object) -> Completion {
    let constructor = ops::get_named(realm, object, "constructor")?;
    if matches!(constructor, Value::Undefined) {
        return Ok(Value::Object(fallback.clone()));
    }
    if !matches!(constructor, Value::Object(_)) {
        return Err(realm.throw(ErrorKind::Type, "object.constructor is not an object"));
    }
    let species = ops::get(
        realm,
        &constructor,
        &Key::Symbol(realm.symbols.species.clone()),
    )?;
    if species.is_nullish() {
        return Ok(Value::Object(fallback.clone()));
    }
    if is_constructor(&species) {
        return Ok(species);
    }
    Err(realm.throw(
        ErrorKind::Type,
        "object.constructor[Symbol.species] is not a constructor",
    ))
}

fn symbol_match_all(
    realm: &mut Realm,
    this: Value,
    args: &[Value],
    constructor: &Object,
    prototype: &Object,
) -> Completion {
    receiver(realm, &this, "@@matchAll")?;
    let string = ops::to_string(realm, &arg(args, 0))?;
    let species = species_constructor(realm, &this, constructor)?;
    let flags = flags_of(realm, &this)?;
    let matcher = ops::construct(
        realm,
        &species,
        &[this.clone(), Value::String(flags.clone())],
    )?;
    let last_index = ops::get(realm, &this, &last_index_key())?;
    let last_index = to_length(realm, &last_index)?;
    set_last_index(realm, &matcher, last_index)?;
    let iterator = StringIterator {
        matcher,
        string,
        global: has_unit(&flags, b'g'),
        unicode: unicode_flags(&flags),
        done: false,
    };
    let kind = Kind::RegExpStringIterator(Rc::new(RefCell::new(iterator)));
    Ok(Value::Object(Object::new(Some(prototype.clone()), kind)))
}

fn iterator_result(realm: &Realm, value: Value, done: bool) -> Value {
    let result = realm.object();
    result.define(crate::string::static_key!("value"), Property::data(value));
    result.define(
        crate::string::static_key!("done"),
        Property::data(Value::Boolean(done)),
    );
    Value::Object(result)
}

fn iterator_next(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let state = match this.as_object().map(|object| match &object.data().kind {
        Kind::RegExpStringIterator(state) => Some(state.clone()),
        _ => None,
    }) {
        Some(Some(state)) => state,
        _ => {
            return Err(realm.throw(
                ErrorKind::Type,
                "Method %RegExpStringIterator%.prototype.next called on incompatible receiver",
            ));
        }
    };
    let (matcher, string, global, unicode, done) = {
        let state = state.borrow();
        (
            state.matcher.clone(),
            state.string.clone(),
            state.global,
            state.unicode,
            state.done,
        )
    };
    if done {
        return Ok(iterator_result(realm, Value::Undefined, true));
    }
    let result = exec(realm, &matcher, &string)?;
    if matches!(result, Value::Null) {
        state.borrow_mut().done = true;
        return Ok(iterator_result(realm, Value::Undefined, true));
    }
    if global {
        step_past_empty(realm, &matcher, &result, &string, unicode)?;
    } else {
        state.borrow_mut().done = true;
    }
    Ok(iterator_result(realm, result, false))
}

/// `GetSubstitution`: a replacement template with `$$`, `$&`, `` $` ``,
/// `$'`, `$n`/`$nn` and `$<name>` references.
pub(crate) fn substitution(
    realm: &mut Realm,
    matched: &[u16],
    string: &[u16],
    position: usize,
    captures: &[Option<JsStr>],
    named: &Value,
    template: &[u16],
) -> Result<Vec<u16>, Thrown> {
    let digit =
        |unit: Option<&u16>| unit.and_then(|unit| char::from_u32(u32::from(*unit))?.to_digit(10));
    let mut result = Vec::new();
    let mut index = 0;
    while index < template.len() {
        let rest = &template[index..];
        if rest[0] != u16::from(b'$') || rest.len() == 1 {
            result.push(rest[0]);
            index += 1;
            continue;
        }
        let next = rest[1];
        if next == u16::from(b'$') {
            result.push(next);
            index += 2;
        } else if next == u16::from(b'&') {
            result.extend_from_slice(matched);
            index += 2;
        } else if next == u16::from(b'`') {
            result.extend_from_slice(&string[..position]);
            index += 2;
        } else if next == u16::from(b'\'') {
            result.extend_from_slice(&string[(position + matched.len()).min(string.len())..]);
            index += 2;
        } else if let Some(first) = digit(rest.get(1)) {
            let mut count = 1;
            let mut number = first as usize;
            if let Some(second) = digit(rest.get(2)) {
                let two = number * 10 + second as usize;
                if two <= captures.len() || two == 0 {
                    (count, number) = (2, two);
                }
            }
            if (1..=captures.len()).contains(&number) {
                if let Some(capture) = &captures[number - 1] {
                    result.extend_from_slice(capture.units());
                }
            } else {
                result.extend_from_slice(&rest[..=count]);
            }
            index += 1 + count;
        } else if next == u16::from(b'<') {
            let close = rest.iter().position(|unit| *unit == u16::from(b'>'));
            match close {
                Some(close) if !matches!(named, Value::Undefined) => {
                    let name = JsStr::from_units(&rest[2..close]);
                    let capture = ops::get(realm, named, &Key::from_js(&name))?;
                    if !matches!(capture, Value::Undefined) {
                        result.extend_from_slice(ops::to_string(realm, &capture)?.units());
                    }
                    index += close + 1;
                }
                _ => {
                    result.extend_from_slice(&rest[..2]);
                    index += 2;
                }
            }
        } else {
            result.push(rest[0]);
            index += 1;
        }
    }
    Ok(result)
}

fn symbol_replace(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    receiver(realm, &this, "@@replace")?;
    let string = ops::to_string(realm, &arg(args, 0))?;
    let mut replacer = arg(args, 1);
    let functional = is_callable(&replacer);
    if !functional {
        replacer = Value::String(ops::to_string(realm, &replacer)?);
    }
    let flags = flags_of(realm, &this)?;
    let global = has_unit(&flags, b'g');
    let unicode = unicode_flags(&flags);
    if global {
        set_last_index(realm, &this, 0)?;
    }
    let mut results = Vec::new();
    loop {
        let result = exec(realm, &this, &string)?;
        if matches!(result, Value::Null) {
            break;
        }
        results.push(result.clone());
        if !global {
            break;
        }
        step_past_empty(realm, &this, &result, &string, unicode)?;
    }
    let units = string.units();
    let mut accumulated = Vec::new();
    let mut next_position = 0;
    for result in results {
        let count = length_of(realm, &result)?.saturating_sub(1);
        let matched = ops::get(realm, &result, &Key::Index(0))?;
        let matched = ops::to_string(realm, &matched)?;
        let position = ops::get_named(realm, &result, "index")?;
        let position = crate::array_builtin::to_integer_or_infinity(realm, &position)?;
        let position = position.clamp(0.0, units.len() as f64) as usize;
        let mut captures = Vec::new();
        for index in 1..=count {
            let capture = ops::get(realm, &result, &crate::array_builtin::index_key(index))?;
            captures.push(match capture {
                Value::Undefined => None,
                capture => Some(ops::to_string(realm, &capture)?),
            });
        }
        let named = ops::get_named(realm, &result, "groups")?;
        let replacement = if functional {
            let mut arguments = vec![Value::String(matched.clone())];
            arguments.extend(
                captures
                    .iter()
                    .map(|capture| capture.clone().map(Value::String).unwrap_or_default()),
            );
            arguments.push(Value::Number(position as f64));
            arguments.push(Value::String(string.clone()));
            if !matches!(named, Value::Undefined) {
                arguments.push(named);
            }
            let value = ops::call(realm, &replacer, Value::Undefined, &arguments)?;
            ops::to_string(realm, &value)?.units().to_vec()
        } else {
            let named = match named {
                Value::Undefined => Value::Undefined,
                named => Value::Object(ops::to_object(realm, &named)?),
            };
            let template = ops::to_string(realm, &replacer)?;
            substitution(
                realm,
                matched.units(),
                units,
                position,
                &captures,
                &named,
                template.units(),
            )?
        };
        if position >= next_position {
            accumulated.extend_from_slice(&units[next_position..position]);
            accumulated.extend_from_slice(&replacement);
            next_position = position + matched.len();
        }
    }
    if next_position < units.len() {
        accumulated.extend_from_slice(&units[next_position..]);
    }
    Ok(Value::String(JsStr::from_units(&accumulated)))
}

fn symbol_search(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    receiver(realm, &this, "@@search")?;
    let string = ops::to_string(realm, &arg(args, 0))?;
    let previous = ops::get(realm, &this, &last_index_key())?;
    let zero = Value::Number(0.0);
    if !crate::object::same_value(&previous, &zero) {
        ops::set(realm, &this, &last_index_key(), zero)?;
    }
    let result = exec(realm, &this, &string)?;
    let current = ops::get(realm, &this, &last_index_key())?;
    if !crate::object::same_value(&current, &previous) {
        ops::set(realm, &this, &last_index_key(), previous)?;
    }
    match result {
        Value::Null => Ok(Value::Number(-1.0)),
        result => ops::get_named(realm, &result, "index"),
    }
}

/// Whether `exec` on a value is the built-in one, so that a split can
/// search ahead instead of trying each position in turn.
fn builtin_exec_of(
    realm: &mut Realm,
    regexp: &Value,
    intrinsic: &Object,
) -> Result<Option<RegExpData>, Thrown> {
    let Some(data) = data_of(regexp) else {
        return Ok(None);
    };
    let method = ops::get_named(realm, regexp, "exec")?;
    Ok(matches!(&method, Value::Object(method) if method == intrinsic).then_some(data))
}

fn symbol_split(
    realm: &mut Realm,
    this: Value,
    args: &[Value],
    constructor: &Object,
    intrinsic_exec: &WeakObject,
) -> Completion {
    receiver(realm, &this, "@@split")?;
    let string = ops::to_string(realm, &arg(args, 0))?;
    let species = species_constructor(realm, &this, constructor)?;
    let flags = flags_of(realm, &this)?;
    let unicode = unicode_flags(&flags);
    let sticky_flags = if has_unit(&flags, b'y') {
        flags.clone()
    } else {
        flags.concat(&JsStr::from("y"))
    };
    let splitter = ops::construct(
        realm,
        &species,
        &[this.clone(), Value::String(sticky_flags)],
    )?;
    let limit = match arg(args, 1) {
        Value::Undefined => u32::MAX,
        limit => ops::to_uint32(realm, &limit)?,
    };
    let mut parts = Vec::new();
    if limit == 0 {
        return Ok(Value::Object(realm.array(parts)));
    }
    let units = string.units();
    let size = units.len() as u64;
    if size == 0 {
        if matches!(exec(realm, &splitter, &string)?, Value::Null) {
            parts.push(Value::String(string));
        }
        return Ok(Value::Object(realm.array(parts)));
    }
    let intrinsic = intrinsic_exec
        .upgrade()
        .expect("RegExp.prototype.exec is alive");
    let fast = builtin_exec_of(realm, &splitter, &intrinsic)?;
    let piece =
        |from: u64, to: u64| Value::String(JsStr::from_units(&units[from as usize..to as usize]));
    let (mut start, mut position) = (0u64, 0u64);
    while position < size {
        let found = match &fast {
            // The built-in sticky exec at each position finds what one
            // search from here finds at its leftmost match.
            Some(data) => match data.pattern.search(units, position as usize) {
                Some(matched) if (matched.range.start as u64) < size => {
                    position = position.max(matched.range.start as u64);
                    set_last_index(realm, &splitter, position)?;
                    builtin_exec(realm, &splitter, data, &string)?
                }
                _ => break,
            },
            None => {
                set_last_index(realm, &splitter, position)?;
                exec(realm, &splitter, &string)?
            }
        };
        if matches!(found, Value::Null) {
            position = advance(units, position, unicode);
            continue;
        }
        let end = ops::get(realm, &splitter, &last_index_key())?;
        let end = to_length(realm, &end)?.min(size);
        if end == start {
            position = advance(units, position, unicode);
            continue;
        }
        parts.push(piece(start, position));
        if parts.len() as u64 == u64::from(limit) {
            return Ok(Value::Object(realm.array(parts)));
        }
        start = end;
        let count = length_of(realm, &found)?.saturating_sub(1);
        for index in 1..=count {
            let capture = ops::get(realm, &found, &crate::array_builtin::index_key(index))?;
            parts.push(capture);
            if parts.len() as u64 == u64::from(limit) {
                return Ok(Value::Object(realm.array(parts)));
            }
        }
        position = start;
    }
    parts.push(piece(start, size));
    Ok(Value::Object(realm.array(parts)))
}

/// `EscapeRegExpPattern`, as V8 writes `source`: `/` outside classes and
/// line terminators are escaped, and the empty pattern is `(?:)`.
fn escape_source(source: &[u16]) -> JsStr {
    if source.is_empty() {
        return JsStr::from("(?:)");
    }
    let mut escaped = Vec::with_capacity(source.len());
    let mut in_class = false;
    let mut index = 0;
    while index < source.len() {
        let unit = source[index];
        let line = match unit {
            0x0A => Some("\\n"),
            0x0D => Some("\\r"),
            0x2028 => Some("\\u2028"),
            0x2029 => Some("\\u2029"),
            _ => None,
        };
        if let Some(line) = line {
            escaped.extend(line.encode_utf16());
        } else if unit == u16::from(b'\\') {
            let next = source.get(index + 1).copied();
            if next.is_some_and(|next| [0x0A, 0x0D, 0x2028, 0x2029].contains(&next)) {
                // An escaped line terminator is written as its escape alone.
                index += 1;
                continue;
            }
            escaped.push(unit);
            if let Some(next) = next {
                escaped.push(next);
                index += 1;
            }
        } else {
            if unit == u16::from(b'/') && !in_class {
                escaped.push(u16::from(b'\\'));
            } else if unit == u16::from(b'[') {
                in_class = true;
            } else if unit == u16::from(b']') {
                in_class = false;
            }
            escaped.push(unit);
        }
        index += 1;
    }
    JsStr::from_units(&escaped)
}

/// The literal text of a regular expression, as its built-in `toString`
/// writes it: escaped source and flags in canonical order.
pub(crate) fn literal_text(data: &RegExpData) -> String {
    let flags: String = FLAGS
        .iter()
        .filter(|flag| data.has(**flag))
        .map(|flag| char::from(*flag))
        .collect();
    format!("/{}/{flags}", escape_source(data.source.units()))
}

/// A flag accessor: the flag of a RegExp object, `undefined` on the
/// prototype itself, and a TypeError for other receivers.
fn flag_getter(realm: &mut Realm, this: &Value, flag: u8, name: &str) -> Completion {
    if let Some(data) = data_of(this) {
        return Ok(Value::Boolean(data.has(flag)));
    }
    if matches!(this, Value::Object(object) if *object == realm.regexp_prototype || *object == realm.protected_regexp_prototype)
    {
        return Ok(Value::Undefined);
    }
    Err(realm.throw(
        ErrorKind::Type,
        &format!("RegExp.prototype.{name} getter called on non-RegExp object"),
    ))
}

fn flags_getter(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    if !matches!(this, Value::Object(_)) {
        return Err(realm.throw(
            ErrorKind::Type,
            "RegExp.prototype.flags getter called on non-object",
        ));
    }
    let mut flags = String::new();
    for (flag, name) in ACCESSORS {
        let value = ops::get(realm, &this, &Key::name(name))?;
        if ops::to_boolean(&value) {
            flags.push(char::from(flag));
        }
    }
    Ok(Value::str(&flags))
}

fn source_getter(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    if let Some(data) = data_of(&this) {
        return Ok(Value::String(escape_source(data.source.units())));
    }
    if matches!(&this, Value::Object(object) if *object == realm.regexp_prototype || *object == realm.protected_regexp_prototype)
    {
        return Ok(Value::str("(?:)"));
    }
    Err(realm.throw(
        ErrorKind::Type,
        "RegExp.prototype.source getter called on non-RegExp object",
    ))
}

/// A boxed native method body.
type Method = Box<dyn Fn(&mut Realm, Value, &[Value]) -> Completion>;

/// The flag accessors in `flags` order.
const ACCESSORS: [(u8, &str); 8] = [
    (b'd', "hasIndices"),
    (b'g', "global"),
    (b'i', "ignoreCase"),
    (b'm', "multiline"),
    (b's', "dotAll"),
    (b'u', "unicode"),
    (b'v', "unicodeSets"),
    (b'y', "sticky"),
];

fn accessor(
    realm: &Realm,
    target: &Object,
    key: Key,
    name: &str,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) {
    let getter = realm.function(&format!("get {name}"), 0, false, code);
    target.define(
        key,
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

fn tag(realm: &Realm, target: &Object, text: &str) {
    target.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str(text),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

pub(crate) fn install(realm: &mut Realm) {
    let constructor = install_on(realm, realm.regexp_prototype.clone());
    realm.set_global("RegExp", Value::Object(constructor));
    realm.protected_regexp_constructor =
        Some(install_on(realm, realm.protected_regexp_prototype.clone()));
}

/// Independent native methods and constructor for one intrinsic prototype.
fn install_on(realm: &mut Realm, prototype: Object) -> Object {
    let self_reference: Rc<RefCell<Option<WeakObject>>> = Rc::default();
    let called = self_reference.clone();
    let constructed = self_reference.clone();
    let constructor = realm.builtin_constructor(
        "RegExp",
        2,
        &prototype,
        move |realm, _, args| {
            let constructor = called
                .borrow()
                .as_ref()
                .and_then(WeakObject::upgrade)
                .expect("RegExp is alive");
            Ok(Value::Object(construct(realm, &constructor, None, args)?))
        },
        move |realm, target, args| {
            let constructor = constructed
                .borrow()
                .as_ref()
                .and_then(WeakObject::upgrade)
                .expect("RegExp is alive");
            construct(realm, &constructor, Some(target), args)
        },
    );
    *self_reference.borrow_mut() = Some(constructor.downgrade());
    accessor(
        realm,
        &constructor,
        Key::Symbol(realm.symbols.species.clone()),
        "[Symbol.species]",
        |_, this, _| Ok(this),
    );
    let exec_function = realm.function("exec", 1, false, |realm, this, args| {
        let Some(data) = data_of(&this) else {
            return Err(realm.throw(
                ErrorKind::Type,
                "Method RegExp.prototype.exec called on incompatible receiver",
            ));
        };
        let string = ops::to_string(realm, &arg(args, 0))?;
        builtin_exec(realm, &this, &data, &string)
    });
    let intrinsic_exec = exec_function.downgrade();
    prototype.define(
        crate::string::static_key!("exec"),
        Property::hidden(Value::Object(exec_function)),
    );
    accessor(
        realm,
        &prototype,
        crate::string::static_key!("dotAll"),
        "dotAll",
        |realm, this, _| flag_getter(realm, &this, b's', "dotAll"),
    );
    accessor(
        realm,
        &prototype,
        crate::string::static_key!("flags"),
        "flags",
        flags_getter,
    );
    for (flag, name) in ACCESSORS {
        if flag == b's' {
            continue;
        }
        accessor(
            realm,
            &prototype,
            Key::name(name),
            name,
            move |realm, this, _| flag_getter(realm, &this, flag, name),
        );
    }
    accessor(
        realm,
        &prototype,
        crate::string::static_key!("source"),
        "source",
        source_getter,
    );
    realm.method(&prototype, "test", 1, |realm, this, args| {
        receiver(realm, &this, "test")?;
        let string = ops::to_string(realm, &arg(args, 0))?;
        Ok(Value::Boolean(!matches!(
            exec(realm, &this, &string)?,
            Value::Null
        )))
    });
    realm.method(&prototype, "toString", 0, |realm, this, _| {
        receiver(realm, &this, "toString")?;
        let source = ops::get_named(realm, &this, "source")?;
        let source = ops::to_string(realm, &source)?;
        let flags = flags_of(realm, &this)?;
        let text = JsStr::from("/")
            .concat(&source)
            .concat(&JsStr::from("/"))
            .concat(&flags);
        Ok(Value::String(text))
    });
    let iterator_prototype = Object::new(Some(realm.iterator_prototype.clone()), Kind::Ordinary);
    realm.method(&iterator_prototype, "next", 0, iterator_next);
    tag(realm, &iterator_prototype, "RegExp String Iterator");
    let symbols = realm.symbols.clone();
    let protocol =
        |realm: &Realm, symbol: &crate::symbol::Symbol, name: &str, length: u32, code: Method| {
            let function = realm.function(
                &format!("[Symbol.{name}]"),
                length,
                false,
                move |realm, this, args| code(realm, this, args),
            );
            prototype.define(
                Key::Symbol(symbol.clone()),
                Property::hidden(Value::Object(function)),
            );
        };
    protocol(realm, &symbols.match_, "match", 1, Box::new(symbol_match));
    let species_fallback = constructor.downgrade();
    protocol(
        realm,
        &symbols.match_all,
        "matchAll",
        1,
        Box::new(move |realm, this, args| {
            let constructor = species_fallback.upgrade().expect("RegExp is alive");
            symbol_match_all(realm, this, args, &constructor, &iterator_prototype)
        }),
    );
    protocol(
        realm,
        &symbols.replace,
        "replace",
        2,
        Box::new(symbol_replace),
    );
    protocol(realm, &symbols.search, "search", 1, Box::new(symbol_search));
    let species_fallback = constructor.downgrade();
    protocol(
        realm,
        &symbols.split,
        "split",
        2,
        Box::new(move |realm, this, args| {
            let constructor = species_fallback.upgrade().expect("RegExp is alive");
            symbol_split(realm, this, args, &constructor, &intrinsic_exec)
        }),
    );
    constructor
}

#[cfg(test)]
mod tests;
