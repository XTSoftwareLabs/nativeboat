// Formatting adapts Node.js algorithms. Copyright Node.js contributors.
// MIT notice: ../../nativeboat-start/src/NOTICES.txt.
//! The `console` object, printing as Node's does: `log`, `info`, `debug`
//! and `dirxml` format their arguments with `util.format` to standard
//! output, `warn`, `error`, `trace` and failed `assert`s to standard error,
//! `dir` inspects one value, groups indent by two spaces, and counters and
//! timers are kept per label. Stack frames are not printed by `trace`, and
//! `table` renders own enumerable rows and columns.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::inspect;
use crate::object::{Key, Property, Slot};
use crate::ops;
use crate::platform;
use crate::realm::Realm;
use crate::string::JsStr;
use crate::value::{Completion, Thrown, Value};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::cell::RefCell;

/// The standard stream a message goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    Out,
    Err,
}

/// The state behind one `console` object.
pub(crate) struct State {
    /// The current group indentation, in spaces.
    indentation: usize,
    counts: BTreeMap<JsStr, u64>,
    timers: BTreeMap<JsStr, f64>,
    /// The clock timers read, in milliseconds.
    clock: fn() -> f64,
    /// Messages kept instead of written, for tests.
    captured: Option<Vec<(Stream, String)>>,
}

impl Default for State {
    fn default() -> State {
        State {
            indentation: 0,
            counts: BTreeMap::new(),
            timers: BTreeMap::new(),
            clock: platform::now,
            captured: None,
        }
    }
}

type Shared = Rc<RefCell<State>>;

fn emit(state: &Shared, stream: Stream, text: String) {
    let mut state = state.borrow_mut();
    match &mut state.captured {
        Some(captured) => captured.push((stream, text)),
        None => {
            match stream {
                Stream::Out => platform::write_stdout(&text),
                Stream::Err => platform::write_stderr(&text),
            };
        }
    }
}

/// Writes a message line, indented by the current group.
fn write(state: &Shared, stream: Stream, text: &str) {
    let indentation = " ".repeat(state.borrow().indentation);
    let mut line = indentation.clone();
    line.push_str(&text.replace('\n', &format!("\n{indentation}")));
    line.push('\n');
    emit(state, stream, line);
}

/// A process warning, as Node's `process.emitWarning` prints one.
fn warning(state: &Shared, message: &str) {
    emit(state, Stream::Err, format!("Warning: {message}\n"));
}

/// A label argument: `default` when missing, otherwise its string.
fn label(realm: &mut Realm, args: &[Value]) -> Result<JsStr, Thrown> {
    match args.first() {
        None | Some(Value::Undefined) => Ok("default".into()),
        Some(label) => ops::to_string(realm, label),
    }
}

/// Node's `formatTime`: milliseconds, seconds, or minutes and seconds.
fn duration(realm: &mut Realm, mut ms: f64) -> Result<String, Thrown> {
    const SECOND: f64 = 1000.0;
    const MINUTE: f64 = 60.0 * SECOND;
    const HOUR: f64 = 60.0 * MINUTE;
    let mut fixed = |value: f64| -> Result<String, Thrown> {
        let text =
            crate::number_builtin::to_fixed(realm, Value::Number(value), &[Value::Number(3.0)])?;
        Ok(ops::to_string(realm, &text)?.to_rust_lossy())
    };
    let (mut hours, mut minutes, mut seconds) = (0u64, 0u64, 0.0);
    if ms >= SECOND {
        if ms >= MINUTE {
            if ms >= HOUR {
                hours = (ms / HOUR) as u64;
                ms = crate::libm::fmod(ms, HOUR);
            }
            minutes = (ms / MINUTE) as u64;
            ms = crate::libm::fmod(ms, MINUTE);
        }
        seconds = ms / SECOND;
    }
    if hours != 0 || minutes != 0 {
        let text = fixed(seconds)?;
        let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
        let lead = if hours != 0 {
            format!("{hours}:{minutes:02}")
        } else {
            format!("{minutes}")
        };
        let unit = if hours != 0 { "h:m" } else { "" };
        return Ok(format!("{lead}:{whole:0>2}.{fraction} ({unit}m:ss.mmm)"));
    }
    if seconds != 0.0 {
        return Ok(format!("{}s", fixed(seconds)?));
    }
    let text = fixed(ms)?;
    let rounded = crate::number::parse(&text.as_str().into());
    Ok(format!("{}ms", crate::number::to_string(rounded)))
}

/// `timeLog` and `timeEnd`: the time since a label's `time`.
fn time_log(
    realm: &mut Realm,
    state: &Shared,
    method: &str,
    args: &[Value],
    end: bool,
) -> Completion {
    let label = label(realm, args)?;
    let started = state.borrow().timers.get(&label).copied();
    let Some(started) = started else {
        warning(
            state,
            &format!("No such label '{label}' for console.{method}()"),
        );
        return Ok(Value::Undefined);
    };
    let elapsed = (state.borrow().clock)() - started;
    let mut message = vec![
        Value::str("%s: %s"),
        Value::String(label.clone()),
        Value::str(&duration(realm, elapsed)?),
    ];
    if !end {
        message.extend(args.iter().skip(1).cloned());
    }
    let text = inspect::format(realm, &message)?;
    write(state, Stream::Out, &text);
    if end {
        state.borrow_mut().timers.remove(&label);
    }
    Ok(Value::Undefined)
}

/// The `depth` option of `console.dir`: a count, or `null` for no limit.
fn depth(realm: &mut Realm, options: Option<&Value>) -> Result<usize, Thrown> {
    let Some(options @ Value::Object(_)) = options else {
        return Ok(inspect::DEPTH);
    };
    Ok(match ops::get_named(realm, options, "depth")? {
        Value::Undefined => inspect::DEPTH,
        Value::Null => usize::MAX,
        depth => {
            let depth = crate::array_builtin::to_integer_or_infinity(realm, &depth)?;
            if depth < 0.0 {
                0
            } else {
                depth.min(usize::MAX as f64) as usize
            }
        }
    })
}

type Method = fn(&mut Realm, &Shared, &[Value]) -> Completion;

fn print(realm: &mut Realm, state: &Shared, stream: Stream, args: &[Value]) -> Completion {
    let text = inspect::format(realm, args)?;
    write(state, stream, &text);
    Ok(Value::Undefined)
}

fn log(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    print(realm, state, Stream::Out, args)
}

fn error(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    print(realm, state, Stream::Err, args)
}

fn dir(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    let depth = depth(realm, args.get(1))?;
    let text = inspect::inspect(realm, args.first().unwrap_or(&Value::Undefined), depth)?;
    write(state, Stream::Out, &text);
    Ok(Value::Undefined)
}

fn trace(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    let message = inspect::format(realm, args)?;
    let text = if message.is_empty() {
        "Trace".to_owned()
    } else {
        format!("Trace: {message}")
    };
    write(state, Stream::Err, &text);
    Ok(Value::Undefined)
}

fn assert(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    if ops::to_boolean(args.first().unwrap_or(&Value::Undefined)) {
        return Ok(Value::Undefined);
    }
    let mut message: Vec<Value> = args.iter().skip(1).cloned().collect();
    match message.first() {
        Some(Value::String(first)) => {
            message[0] = Value::str(&format!("Assertion failed: {}", first.to_rust_lossy()));
        }
        _ => message.insert(0, Value::str("Assertion failed")),
    }
    error(realm, state, &message)
}

fn count(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    let label = label(realm, args)?;
    let count = {
        let mut state = state.borrow_mut();
        let count = state.counts.entry(label.clone()).or_insert(0);
        *count += 1;
        *count
    };
    write(state, Stream::Out, &format!("{label}: {count}"));
    Ok(Value::Undefined)
}

fn count_reset(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    let label = label(realm, args)?;
    let found = state.borrow_mut().counts.remove(&label).is_some();
    if !found {
        warning(state, &format!("Count for '{label}' does not exist"));
    }
    Ok(Value::Undefined)
}

fn group(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    if !args.is_empty() {
        log(realm, state, args)?;
    }
    state.borrow_mut().indentation += 2;
    Ok(Value::Undefined)
}

fn group_end(_: &mut Realm, state: &Shared, _: &[Value]) -> Completion {
    let mut state = state.borrow_mut();
    state.indentation = state.indentation.saturating_sub(2);
    Ok(Value::Undefined)
}

fn time(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    let label = label(realm, args)?;
    if state.borrow().timers.contains_key(&label) {
        warning(
            state,
            &format!("Label '{label}' already exists for console.time()"),
        );
        return Ok(Value::Undefined);
    }
    let now = (state.borrow().clock)();
    state.borrow_mut().timers.insert(label, now);
    Ok(Value::Undefined)
}

fn time_end(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    time_log(realm, state, "timeEnd", args, true)
}

fn time_log_method(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    time_log(realm, state, "timeLog", args, false)
}

fn table(realm: &mut Realm, state: &Shared, args: &[Value]) -> Completion {
    let properties = args
        .get(1)
        .filter(|value| !matches!(value, Value::Undefined));
    if properties.is_some_and(|value| !value.as_object().is_some_and(|object| object.is_array())) {
        return Err(realm.throw(
            crate::realm::ErrorKind::Type,
            "The 'properties' argument must be an Array",
        ));
    }
    let Some(Value::Object(data)) = args.first() else {
        return log(realm, state, args.get(..1).unwrap_or(&[]));
    };
    let keys = crate::json_builtin::enumerable_keys(realm, data)?;
    let holder = Value::Object(data.clone());
    let mut rows = Vec::new();
    let mut columns = Vec::new();
    if let Some(properties) = properties {
        let length = crate::array_builtin::length_of(realm, properties)?;
        for index in 0..length {
            let value = ops::get(realm, properties, &crate::array_builtin::index_key(index))?;
            let key = Key::from_js(&ops::to_string(realm, &value)?);
            if !columns.contains(&key) {
                columns.push(key);
            }
        }
    }
    let mut values = false;
    for key in &keys {
        let row = ops::get(realm, &holder, key)?;
        if let Value::Object(object) = &row {
            if properties.is_none() {
                for key in crate::json_builtin::enumerable_keys(realm, object)? {
                    if !columns.contains(&key) {
                        columns.push(key);
                    }
                }
            }
        } else {
            values = true;
        }
        rows.push(row);
    }
    let values = values && properties.is_none();
    let mut grid = vec![vec!["(index)".to_owned()]];
    grid[0].extend(columns.iter().map(|key| key.to_js().to_rust_lossy()));
    if values {
        grid[0].push("Values".to_owned());
    }
    for (key, row) in keys.iter().zip(rows) {
        let mut cells = vec![key.to_js().to_rust_lossy()];
        for column in &columns {
            let cell = match &row {
                Value::Object(object) if object.own_property(column).is_some() => {
                    let value = ops::get(realm, &row, column)?;
                    inspect::inspect(realm, &value, 0)?
                }
                _ => String::new(),
            };
            cells.push(cell);
        }
        if values {
            cells.push(if matches!(row, Value::Object(_)) {
                String::new()
            } else {
                inspect::inspect(realm, &row, 0)?
            });
        }
        grid.push(cells);
    }
    let widths: Vec<usize> = (0..grid[0].len())
        .map(|column| {
            grid.iter()
                .map(|row| row[column].encode_utf16().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let border = |left: &str, join: &str, right: &str| {
        format!(
            "{left}{}{right}",
            widths
                .iter()
                .map(|width| "─".repeat(width + 2))
                .collect::<Vec<_>>()
                .join(join)
        )
    };
    let mut lines = vec![border("┌", "┬", "┐")];
    for (index, row) in grid.iter().enumerate() {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(text, width)| {
                format!(
                    " {text}{} ",
                    " ".repeat(width.saturating_sub(text.encode_utf16().count()))
                )
            })
            .collect();
        lines.push(format!("│{}│", cells.join("│")));
        if index == 0 {
            lines.push(border("├", "┼", "┤"));
        }
    }
    lines.push(border("└", "┴", "┘"));
    write(state, Stream::Out, &lines.join("\n"));
    Ok(Value::Undefined)
}

fn nothing(_: &mut Realm, _: &Shared, _: &[Value]) -> Completion {
    Ok(Value::Undefined)
}

/// The methods in Node's order.
const METHODS: [(&str, Method); 22] = [
    ("log", log),
    ("info", log),
    ("debug", log),
    ("warn", error),
    ("error", error),
    ("dir", dir),
    ("time", time),
    ("timeEnd", time_end),
    ("timeLog", time_log_method),
    ("trace", trace),
    ("assert", assert),
    ("clear", nothing),
    ("count", count),
    ("countReset", count_reset),
    ("group", group),
    ("groupEnd", group_end),
    ("table", table),
    ("dirxml", log),
    ("groupCollapsed", group),
    ("profile", nothing),
    ("profileEnd", nothing),
    ("timeStamp", nothing),
];

/// Defines `console` on the global object with the given state.
pub(crate) fn install_with(realm: &mut Realm, state: Shared) {
    let console = realm.object();
    for (name, method) in METHODS {
        let state = state.clone();
        let function = realm.function(name, 0, false, move |realm, _, args| {
            method(realm, &state, args)
        });
        console.define(Key::name(name), Property::data(Value::Object(function)));
    }
    console.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str("console"),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
    realm.global.define(
        Key::name("console"),
        Property::hidden(Value::Object(console)),
    );
}

pub(crate) fn install(realm: &mut Realm) {
    install_with(realm, Shared::default());
}

#[cfg(test)]
mod tests;
