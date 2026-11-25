//! Node's global `performance`: milliseconds since the realm started, at
//! the monotonic clock's resolution, the wall-clock time it started, and
//! user timing marks and measures. Pages replace it with the window's.

use crate::prelude::*;

use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;

pub(crate) fn install(realm: &mut Realm) {
    let clock = realm.event_loop.clock;
    let origin = clock();
    let now = move || clock() - origin;
    let performance = realm.object();
    let entries: Rc<RefCell<Vec<Object>>> = Rc::default();
    performance.define(
        Key::name("timeOrigin"),
        Property::data(Value::Number(crate::date_builtin::now())),
    );
    let method = |realm: &mut Realm, name: &str, length: u32, body: Box<Method>| {
        let function = realm.function(name, length, false, body);
        performance.define(Key::name(name), Property::hidden(Value::Object(function)));
    };
    method(
        realm,
        "now",
        0,
        Box::new(move |_, _, _| Ok(Value::Number(now()))),
    );
    let marks = entries.clone();
    method(
        realm,
        "mark",
        1,
        Box::new(move |realm, _, args| {
            let name = required_name(realm, args)?;
            let options = args.get(1).cloned().unwrap_or(Value::Undefined);
            let start = match option(realm, &options, "startTime")? {
                Value::Undefined => now(),
                value => ops::to_number(realm, &value)?,
            };
            let detail = match option(realm, &options, "detail")? {
                Value::Undefined => Value::Null,
                value => value,
            };
            let entry = self::entry(realm, &name, "mark", start, 0.0, detail);
            marks.borrow_mut().push(entry.clone());
            Ok(Value::Object(entry))
        }),
    );
    let measures = entries.clone();
    method(
        realm,
        "measure",
        1,
        Box::new(move |realm, _, args| {
            let name = required_name(realm, args)?;
            let first = args.get(1).cloned().unwrap_or(Value::Undefined);
            // A mark's start, a number as is, or else the default.
            let time = |realm: &mut Realm, value: Value, default: f64| -> Result<f64, Thrown> {
                match value {
                    Value::Undefined => Ok(default),
                    Value::Number(number) => Ok(number),
                    other => {
                        let wanted = ops::to_string(realm, &other)?.to_rust_lossy();
                        let found = measures.borrow().iter().rev().find_map(|entry| {
                            let entry = Value::Object(entry.clone());
                            let kind = string_of(realm, &entry, "entryType");
                            (string_of(realm, &entry, "name") == wanted && kind == "mark")
                                .then(|| ops::get_named(realm, &entry, "startTime"))
                        });
                        match found {
                            Some(Ok(Value::Number(start))) => Ok(start),
                            _ => Err(realm.throw(
                                ErrorKind::Syntax,
                                &format!("The \"{wanted}\" performance mark has not been set"),
                            )),
                        }
                    }
                }
            };
            let (start, end, detail) = if matches!(first, Value::Object(_)) {
                let start = option(realm, &first, "start")?;
                let end = option(realm, &first, "end")?;
                let duration = option(realm, &first, "duration")?;
                let detail = option(realm, &first, "detail")?;
                let start = time(realm, start, 0.0)?;
                let end = match duration {
                    Value::Undefined => time(realm, end, now())?,
                    duration => start + ops::to_number(realm, &duration)?,
                };
                (start, end, detail)
            } else {
                let start = time(realm, first, 0.0)?;
                let end = args.get(2).cloned().unwrap_or(Value::Undefined);
                (start, time(realm, end, now())?, Value::Undefined)
            };
            let detail = if matches!(detail, Value::Undefined) {
                Value::Null
            } else {
                detail
            };
            let entry = self::entry(realm, &name, "measure", start, end - start, detail);
            measures.borrow_mut().push(entry.clone());
            Ok(Value::Object(entry))
        }),
    );
    let all = entries.clone();
    method(
        realm,
        "getEntries",
        0,
        Box::new(move |realm, _, _| {
            let list = all.borrow().iter().cloned().map(Value::Object).collect();
            Ok(Value::Object(realm.array(list)))
        }),
    );
    for (name, by_name) in [("getEntriesByName", true), ("getEntriesByType", false)] {
        let listed = entries.clone();
        method(
            realm,
            name,
            1,
            Box::new(move |realm, _, args| {
                let wanted = ops::to_string(realm, &args.first().cloned().unwrap_or_default())?
                    .to_rust_lossy();
                let kind = match args.get(1) {
                    Some(kind) if by_name && !matches!(kind, Value::Undefined) => {
                        Some(ops::to_string(realm, kind)?.to_rust_lossy())
                    }
                    _ => None,
                };
                let entries = listed.borrow().clone();
                let mut list = Vec::new();
                for entry in entries {
                    let value = Value::Object(entry);
                    let (name, entry_type) = (
                        string_of(realm, &value, "name"),
                        string_of(realm, &value, "entryType"),
                    );
                    let matches = if by_name {
                        name == wanted && kind.as_ref().is_none_or(|kind| *kind == entry_type)
                    } else {
                        entry_type == wanted
                    };
                    if matches {
                        list.push(value);
                    }
                }
                Ok(Value::Object(realm.array(list)))
            }),
        );
    }
    for (name, kind) in [("clearMarks", "mark"), ("clearMeasures", "measure")] {
        let cleared = entries.clone();
        method(
            realm,
            name,
            0,
            Box::new(move |realm, _, args| {
                let only = match args.first() {
                    None | Some(Value::Undefined) => None,
                    Some(name) => Some(ops::to_string(realm, name)?.to_rust_lossy()),
                };
                let entries = cleared.borrow().clone();
                let mut kept = Vec::new();
                for entry in entries {
                    let value = Value::Object(entry.clone());
                    let removed = string_of(realm, &value, "entryType") == kind
                        && only
                            .as_ref()
                            .is_none_or(|only| string_of(realm, &value, "name") == *only);
                    if !removed {
                        kept.push(entry);
                    }
                }
                *cleared.borrow_mut() = kept;
                Ok(Value::Undefined)
            }),
        );
    }
    let json = performance.clone();
    method(
        realm,
        "toJSON",
        0,
        Box::new(move |realm, _, _| {
            let result = realm.object();
            let origin = ops::get_named(realm, &Value::Object(json.clone()), "timeOrigin")?;
            result.define(Key::name("timeOrigin"), Property::data(origin));
            Ok(Value::Object(result))
        }),
    );
    realm.set_global("performance", Value::Object(performance));
}

type Method = dyn Fn(&mut Realm, Value, &[Value]) -> Result<Value, Thrown>;

/// The entry name, which Node requires.
fn required_name(realm: &mut Realm, args: &[Value]) -> Result<String, Thrown> {
    match args.first() {
        None | Some(Value::Undefined) => {
            let error = realm.error(ErrorKind::Type, "The \"name\" argument must be specified");
            error.define(
                Key::name("code"),
                Property::data(Value::str("ERR_MISSING_ARGS")),
            );
            Err(Thrown(Value::Object(error)))
        }
        Some(name) => Ok(ops::to_string(realm, name)?.to_rust_lossy()),
    }
}

/// An option of an options object, or undefined.
fn option(realm: &mut Realm, options: &Value, name: &str) -> Result<Value, Thrown> {
    match options {
        Value::Object(_) => ops::get(realm, options, &Key::name(name)),
        _ => Ok(Value::Undefined),
    }
}

fn string_of(realm: &mut Realm, entry: &Value, name: &str) -> String {
    match ops::get(realm, entry, &Key::name(name)) {
        Ok(Value::String(text)) => text.to_rust_lossy(),
        _ => String::new(),
    }
}

fn entry(
    realm: &mut Realm,
    name: &str,
    kind: &str,
    start: f64,
    duration: f64,
    detail: Value,
) -> Object {
    let entry = realm.object();
    entry.define(Key::name("name"), Property::data(Value::str(name)));
    entry.define(Key::name("entryType"), Property::data(Value::str(kind)));
    entry.define(Key::name("startTime"), Property::data(Value::Number(start)));
    entry.define(
        Key::name("duration"),
        Property::data(Value::Number(duration)),
    );
    entry.define(Key::name("detail"), Property::data(detail));
    entry
}

#[cfg(test)]
mod tests;
