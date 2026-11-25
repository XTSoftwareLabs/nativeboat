//! Node's coded errors (`internal/errors`): errors whose prototype, shared by
//! every error with the same code, gives `toString` as `Name [CODE]:
//! message` and reports the base class as `constructor`, with an enumerable
//! own `code`, and Node 22's argument validation messages.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Slot, Thrown, Value, ops};

/// A Node error of `kind` with `code` and `message`, as Node's `NodeError`
/// classes create them.
pub fn node_error(realm: &mut Realm, kind: ErrorKind, code: &str, message: &str) -> Thrown {
    Thrown(Value::Object(object(realm, kind, code, message)))
}

/// The coded error object, before a binding adds any operation-specific fields
/// and wraps it in a thrown completion.
pub(crate) fn object(realm: &mut Realm, kind: ErrorKind, code: &str, message: &str) -> Object {
    let prototype = prototype(realm, kind, code);
    let error = Object::new(Some(prototype), Kind::Error);
    crate::errors::capture_stack(realm, &error);
    error.define(Key::name("code"), Property::data(Value::str(code)));
    error.define(Key::name("message"), Property::hidden(Value::str(message)));
    error
}

/// The prototype of `NodeError` for a code: `toString` and a `constructor`
/// getter returning the base error constructor.
fn prototype(realm: &mut Realm, kind: ErrorKind, code: &str) -> Object {
    if let Some((_, _, prototype)) = realm
        .node_error_prototypes
        .iter()
        .find(|(known, name, _)| *known == kind && name == code)
    {
        return prototype.clone();
    }
    let base = realm.error_prototype_for(kind).clone();
    let prototype = Object::new(Some(base.clone()), Kind::Ordinary);
    let key = code.to_owned();
    let to_string = realm.function("toString", 0, false, move |realm, this, _| {
        let name = ops::get_named(realm, &this, "name")?;
        let name = ops::to_string(realm, &name)?.to_rust_lossy();
        let message = ops::get_named(realm, &this, "message")?;
        let message = ops::to_string(realm, &message)?.to_rust_lossy();
        Ok(Value::str(&format!("{name} [{key}]: {message}")))
    });
    prototype.define(
        Key::name("toString"),
        Property::hidden(Value::Object(to_string)),
    );
    let constructor =
        base.own_property(&Key::name("constructor"))
            .map_or(Value::Undefined, |property| match property.slot {
                Slot::Data { value, .. } => value,
                Slot::Accessor { .. } => Value::Undefined,
            });
    let getter = realm.function("get constructor", 0, false, move |_, _, _| {
        Ok(constructor.clone())
    });
    prototype.define(
        Key::name("constructor"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    realm
        .node_error_prototypes
        .push((kind, code.to_owned(), prototype.clone()));
    prototype
}

/// Node's primitive type names in `ERR_INVALID_ARG_TYPE` expectations.
const TYPES: &[&str] = &[
    "string", "function", "number", "object", "Function", "Object", "boolean", "bigint", "symbol",
];

/// Node's `formatList`: `A`, `A or B`, `A, B, or C`.
pub(crate) fn list(items: &[String], conjunction: &str) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} {conjunction} {second}"),
        [rest @ .., last] => format!("{}, {conjunction} {last}", rest.join(", ")),
    }
}

/// `ERR_INVALID_ARG_TYPE`: `expected` lists accepted types (`string`),
/// classes (`Error`) or other descriptions, as Node's message builder
/// formats them.
pub fn invalid_arg_type(
    realm: &mut Realm,
    name: &str,
    expected: &[&str],
    actual: &Value,
) -> Thrown {
    let received = match specific_type(realm, actual) {
        Ok(received) => received,
        Err(thrown) => return thrown,
    };
    let mut message = if name.ends_with(" argument") {
        format!("The {name} must be ")
    } else {
        let kind = if name.contains('.') {
            "property"
        } else {
            "argument"
        };
        format!("The \"{name}\" {kind} must be ")
    };
    let mut types = Vec::new();
    let mut instances = Vec::new();
    let mut other = Vec::new();
    for value in expected {
        if TYPES.contains(value) {
            types.push(value.to_lowercase());
        } else if is_class_name(value) {
            instances.push((*value).to_owned());
        } else {
            other.push((*value).to_owned());
        }
    }
    if !instances.is_empty()
        && let Some(position) = types.iter().position(|kind| kind == "object")
    {
        types.remove(position);
        instances.push("Object".to_owned());
    }
    if !types.is_empty() {
        let of = if types.len() > 1 {
            "one of type"
        } else {
            "of type"
        };
        message.push_str(&format!("{of} {}", list(&types, "or")));
        if !instances.is_empty() || !other.is_empty() {
            message.push_str(" or ");
        }
    }
    if !instances.is_empty() {
        message.push_str(&format!("an instance of {}", list(&instances, "or")));
        if !other.is_empty() {
            message.push_str(" or ");
        }
    }
    match other.as_slice() {
        [] => {}
        [only] => {
            if only.to_lowercase() != *only {
                message.push_str("an ");
            }
            message.push_str(only);
        }
        _ => message.push_str(&format!("one of {}", list(&other, "or"))),
    }
    message.push_str(&format!(". Received {received}"));
    node_error(realm, ErrorKind::Type, "ERR_INVALID_ARG_TYPE", &message)
}

/// Node's `classRegExp`: `/^[A-Z][a-zA-Z0-9]*$/`.
fn is_class_name(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
        && characters.all(|character| character.is_ascii_alphanumeric())
}

/// `ERR_OUT_OF_RANGE` for a received value outside `range`.
pub fn out_of_range(realm: &mut Realm, name: &str, range: &str, value: &Value) -> Thrown {
    let text = match received(realm, value) {
        Ok(text) => text,
        Err(thrown) => return thrown,
    };
    let message =
        format!("The value of \"{name}\" is out of range. It must be {range}. Received {text}");
    node_error(realm, ErrorKind::Range, "ERR_OUT_OF_RANGE", &message)
}

/// How `ERR_OUT_OF_RANGE` shows a received value: integers beyond 2^32
/// with digit separators, others as `util.inspect` shows them.
pub(crate) fn received(realm: &mut Realm, value: &Value) -> Result<String, Thrown> {
    match value {
        Value::Number(number)
            if crate::float::fract(*number) == 0.0 && number.abs() > 4_294_967_296.0 =>
        {
            Ok(separated(&crate::number::to_string(*number)))
        }
        _ => crate::inspect::inspect(realm, value, crate::inspect::DEPTH),
    }
}

/// `ERR_INVALID_ARG_VALUE`: the value as `util.inspect` shows it, cut at
/// 128 characters.
pub fn invalid_arg_value(realm: &mut Realm, name: &str, value: &Value, reason: &str) -> Thrown {
    let mut inspected = match crate::inspect::inspect(realm, value, crate::inspect::DEPTH) {
        Ok(text) => text,
        Err(thrown) => return thrown,
    };
    let units: Vec<u16> = inspected.encode_utf16().collect();
    if units.len() > 128 {
        inspected = String::from_utf16_lossy(&units[..128]) + "...";
    }
    let kind = if name.contains('.') {
        "property"
    } else {
        "argument"
    };
    let message = format!("The {kind} '{name}' {reason}. Received {inspected}");
    node_error(realm, ErrorKind::Type, "ERR_INVALID_ARG_VALUE", &message)
}

/// Node's `addNumericalSeparator`: underscores before each group of three
/// trailing characters, keeping at least one character before the first.
fn separated(text: &str) -> String {
    let start = usize::from(text.starts_with('-'));
    let mut end = text.len();
    let mut groups = String::new();
    while end >= start + 4 {
        groups.insert_str(0, &format!("_{}", &text[end - 3..end]));
        end -= 3;
    }
    format!("{}{groups}", &text[..end])
}

/// Node's `determineSpecificType`, describing a received value.
pub fn specific_type(realm: &mut Realm, value: &Value) -> Result<String, Thrown> {
    Ok(match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Boolean(flag) => format!("type boolean ({flag})"),
        Value::BigInt(value) => format!("type bigint ({}n)", value.to_radix(10)),
        Value::Number(number) => {
            let text = if *number == 0.0 && number.is_sign_negative() {
                "-0".to_owned()
            } else {
                crate::number::to_string(*number)
            };
            format!("type number ({text})")
        }
        Value::String(text) => {
            let mut units = text.units().to_vec();
            if units.len() > 28 {
                units.truncate(25);
                units.extend("...".encode_utf16());
            }
            if units.contains(&u16::from(b'\'')) {
                format!("type string ({})", crate::json_builtin::quote(&units))
            } else {
                format!("type string ('{}')", String::from_utf16_lossy(&units))
            }
        }
        Value::Symbol(_) => {
            let text = crate::inspect::inspect(realm, value, 0)?;
            format!("type symbol ({text})")
        }
        Value::Object(object) => {
            if ops::type_of(value) == "function" {
                let name = ops::get_named(realm, value, "name")?;
                let name = ops::to_string(realm, &name)?.to_rust_lossy();
                return Ok(format!("function {name}"));
            }
            let constructor = ops::get_named(realm, value, "constructor")?;
            if ops::to_boolean(&constructor)
                && ops::in_operator(realm, &Value::str("name"), &constructor)?
            {
                let name = ops::get_named(realm, &constructor, "name")?;
                let name = ops::to_string(realm, &name)?.to_rust_lossy();
                return Ok(format!("an instance of {name}"));
            }
            // `util.inspect` at depth -1: empty objects in full, others
            // abbreviated.
            let empty = object.own_keys().iter().all(|key| {
                !object
                    .own_property(key)
                    .is_some_and(|property| property.enumerable)
            });
            match (
                object.data().prototype.is_none(),
                &object.data().kind,
                empty,
            ) {
                (true, _, true) => "[Object: null prototype] {}",
                (true, _, false) => "[Object: null prototype]",
                (false, Kind::Array(_), true) => "[]",
                (false, Kind::Array(_), false) => "[Array]",
                (false, _, true) => "{}",
                (false, _, false) => "[Object]",
            }
            .to_owned()
        }
    })
}

#[cfg(test)]
mod tests;
