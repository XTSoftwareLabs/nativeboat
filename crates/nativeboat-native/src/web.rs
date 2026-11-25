//! The Web APIs of renderer pages. NativeBoat compiles `nativeboat-web`'s
//! JavaScript prelude into each renderer graph. The prelude defines `window`,
//! `document`, nodes, events, ranges and editing over the natives here. The
//! document lives in a Rust [`Dom`], and scripts reach it through node ids.
//! A [`Page`] answers what needs layout or the platform.
//!
//! The prelude is one function of `host`, `native` and `page`. Before a
//! renderer's modules run, [`Realm::install_web`] creates those three
//! objects and leaves them for the prelude, which runs first.

mod dom_ops;
mod history;
mod media;
mod page;

pub use media::{Frame, Media, MediaSource};
pub use page::{BlankPage, Page, Rect, ScrollAlign};

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Completion, ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops, platform};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use nativeboat_dom::{Dom, DomError, NodeId};

/// The global through which the prelude receives its arguments.
pub const PRELUDE_ARGUMENTS: &str = "__nativeboat_web";

/// A native over the document.
type NativeOp<T> = fn(&T, &mut Realm, &[Value]) -> Completion;
type DomOp = NativeOp<RefCell<Dom>>;

/// A native over the page.
type PageOp = NativeOp<dyn Page>;

fn arg(args: &[Value], index: usize) -> &Value {
    args.get(index).unwrap_or(&Value::Undefined)
}

fn node_at(realm: &Realm, dom: &Dom, args: &[Value], index: usize) -> Result<NodeId, Thrown> {
    match arg(args, index) {
        Value::Number(number)
            if crate::float::fract(*number) == 0.0
                && *number >= 0.0
                && *number <= f64::from(u32::MAX)
                && dom.exists(NodeId(*number as u32)) =>
        {
            Ok(NodeId(*number as u32))
        }
        _ => Err(realm.throw(ErrorKind::Type, "not a node")),
    }
}

fn node(realm: &Realm, dom: &RefCell<Dom>, args: &[Value], index: usize) -> Result<NodeId, Thrown> {
    node_at(realm, &dom.borrow(), args, index)
}

fn optional_node(
    realm: &Realm,
    dom: &RefCell<Dom>,
    args: &[Value],
    index: usize,
) -> Result<Option<NodeId>, Thrown> {
    if matches!(arg(args, index), Value::Undefined | Value::Null) {
        return Ok(None);
    }
    node(realm, dom, args, index).map(Some)
}

fn text(realm: &mut Realm, args: &[Value], index: usize) -> Result<String, Thrown> {
    Ok(ops::to_string(realm, arg(args, index))?.to_rust_lossy())
}

fn number(realm: &mut Realm, args: &[Value], index: usize) -> Result<f64, Thrown> {
    ops::to_number(realm, arg(args, index))
}

fn flag(args: &[Value], index: usize) -> bool {
    ops::to_boolean(arg(args, index))
}

fn string(value: &str) -> Value {
    Value::str(value)
}

fn optional_string(value: Option<&str>) -> Value {
    value.map_or(Value::Null, string)
}

fn id(value: Option<NodeId>) -> Value {
    value.map_or(Value::Null, |id| Value::Number(f64::from(id.0)))
}

fn ids(realm: &Realm, values: Vec<NodeId>) -> Value {
    let values = values
        .into_iter()
        .map(|id| Value::Number(f64::from(id.0)))
        .collect();
    Value::Object(realm.array(values))
}

fn numbers(realm: &Realm, values: &[f32]) -> Value {
    let values = values
        .iter()
        .map(|value| Value::Number(f64::from(*value)))
        .collect();
    Value::Object(realm.array(values))
}

fn pairs(realm: &Realm, values: Vec<(String, String)>) -> Value {
    let values = values
        .into_iter()
        .map(|(name, value)| Value::Object(realm.array(vec![string(&name), string(&value)])))
        .collect();
    Value::Object(realm.array(values))
}

/// A DOM error, thrown with the exception name in front of the message; the
/// prelude turns it into a `DOMException`.
fn failure(realm: &Realm, error: DomError) -> Thrown {
    realm.throw(
        ErrorKind::Error,
        &format!("{}: {}", error.name(), error.message()),
    )
}

/// `undefined`, for natives that return nothing.
fn unit() -> Completion {
    Ok(Value::Undefined)
}

fn done<T>(realm: &Realm, result: Result<T, DomError>) -> Completion {
    result
        .map(|_| Value::Undefined)
        .map_err(|error| failure(realm, error))
}

/// The items of an array-like value as strings, or `None` for a non-object.
fn strings(realm: &mut Realm, value: &Value) -> Result<Option<Vec<String>>, Thrown> {
    if !matches!(value, Value::Object(_)) {
        return Ok(None);
    }
    let length = ops::get_named(realm, value, "length")?;
    let length = ops::to_number(realm, &length)?;
    let mut items = Vec::new();
    let mut index = 0u32;
    while f64::from(index) < length {
        let item = ops::get(realm, value, &Key::Index(index))?;
        items.push(ops::to_string(realm, &item)?.to_rust_lossy());
        index += 1;
    }
    Ok(Some(items))
}

/// An object of named native functions.
fn natives<T: ?Sized + 'static>(
    realm: &Realm,
    target: &Rc<T>,
    operations: &[(&str, NativeOp<T>)],
) -> Object {
    let object = Object::new(None, crate::Kind::Ordinary);
    for (name, operation) in operations {
        let target = target.clone();
        let operation = *operation;
        let function = realm.function(name, 0, false, move |realm, _, args| {
            operation(&target, realm, args)
        });
        object.define(Key::name(name), Property::data(Value::Object(function)));
    }
    object
}

pub use crate::electron_builtin::chrome_major;
pub(crate) use crate::electron_builtin::user_agent;

/// The host object of the prelude: a clock, the locale, console output and
/// error reports. The prelude adds `web`, the renderer's interface, and
/// `onerror`.
fn host(realm: &Realm) -> Object {
    let host = realm.object();
    let now = realm.function("now", 0, false, |_, _, _| {
        Ok(Value::Number(platform::now()))
    });
    let locale = realm.function("locale", 0, false, |_, _, _| {
        Ok(Value::str(&platform::user_locale()))
    });
    let languages = realm.function("languages", 0, false, |realm, _, _| {
        let languages = platform::accept_languages()
            .iter()
            .map(|language| Value::str(language))
            .collect();
        Ok(Value::Object(realm.array(languages)))
    });
    let user_agent = realm.function("userAgent", 0, false, |realm, _, _| {
        Ok(Value::str(&user_agent(realm)))
    });
    let processors = realm.function("processors", 0, false, |_, _, _| {
        Ok(Value::Number(f64::from(
            crate::os_binding::processor_count(),
        )))
    });
    let print = realm.function("print", 2, false, |realm, _, args| {
        let level = text(realm, args, 0)?;
        let message = text(realm, args, 1)?;
        let line = format!("{message}\n");
        if level == "log" || level == "info" {
            platform::write_stdout(&line);
        } else {
            platform::write_stderr(&line);
        }
        Ok(Value::Undefined)
    });
    let report = realm.function("report", 2, false, |realm, _, args| {
        report(realm, arg(args, 0));
        Ok(Value::Undefined)
    });
    // The document's title changed: an Electron window learns it in its
    // main realm, after the renderer's current task.
    let title = realm.function("title", 1, false, |realm, _, args| {
        let title = ops::to_string(realm, arg(args, 0))?;
        let main = realm.electron_ipc_main;
        let window = realm.electron_window;
        if main != 0 && window != 0 && main != realm as *mut Realm as usize {
            // SAFETY: the generated host owns the main realm for the
            // renderer's lifetime, on this thread; queueing a job does not
            // run main-realm code from here.
            let main = unsafe { &mut *(main as *mut Realm) };
            main.enqueue(Box::new(move |main| {
                crate::electron_window::page_title_updated(main, window, &title)
            }));
        }
        Ok(Value::Undefined)
    });
    // A context menu no page script cancelled: an Electron window's
    // `webContents` emits `context-menu` with its parameters in its main
    // realm, after the renderer's current task.
    let context_menu = realm.function("contextMenu", 1, false, |realm, _, args| {
        let main = realm.electron_ipc_main;
        let window = realm.electron_window;
        if main != 0 && window != 0 && main != realm as *mut Realm as usize {
            // SAFETY: as for `title`; cloning makes objects in the main
            // realm without running its code.
            let main = unsafe { &mut *(main as *mut Realm) };
            let params = crate::structured_clone_builtin::clone_between(realm, main, arg(args, 0))?;
            main.enqueue(Box::new(move |main| {
                crate::electron_window::context_menu(main, window, params)
            }));
        }
        Ok(Value::Undefined)
    });
    // A notification the page shows, by its number: an Electron window's
    // main realm shows it, after the renderer's current task. False when
    // the page has no main realm to show it.
    let notify = realm.function("notify", 5, false, |realm, _, args| {
        let notification = ops::to_number(realm, arg(args, 0))? as u32;
        let title = ops::to_string(realm, arg(args, 1))?.units().to_vec();
        let body = ops::to_string(realm, arg(args, 2))?.units().to_vec();
        let silent = ops::to_boolean(arg(args, 3));
        // The icon's width, height and RGBA pixels, as `imagePixels` gives.
        let pixels = match arg(args, 4) {
            Value::Object(_) => {
                let icon = arg(args, 4).clone();
                let width = ops::get(realm, &icon, &Key::Index(0))?;
                let height = ops::get(realm, &icon, &Key::Index(1))?;
                let bytes = ops::get(realm, &icon, &Key::Index(2))?;
                let width = ops::to_number(realm, &width)? as u32;
                let height = ops::to_number(realm, &height)? as u32;
                crate::typed_array_builtin::view_bytes(&bytes).map(|bytes| (width, height, bytes))
            }
            _ => None,
        };
        let main = realm.electron_ipc_main;
        let window = realm.electron_window;
        if main == 0 || window == 0 || main == realm as *mut Realm as usize {
            return Ok(Value::Boolean(false));
        }
        // SAFETY: as for `title`.
        let main = unsafe { &mut *(main as *mut Realm) };
        main.enqueue(Box::new(move |main| {
            crate::electron_notify::show_page(
                main,
                window,
                notification,
                &title,
                &body,
                silent,
                pixels,
            )
        }));
        Ok(Value::Boolean(true))
    });
    let close_notification = realm.function("closeNotification", 1, false, |realm, _, args| {
        let notification = ops::to_number(realm, arg(args, 0))? as u32;
        let main = realm.electron_ipc_main;
        let window = realm.electron_window;
        if main != 0 && window != 0 && main != realm as *mut Realm as usize {
            // SAFETY: as for `title`.
            let main = unsafe { &mut *(main as *mut Realm) };
            main.enqueue(Box::new(move |main| {
                crate::electron_notify::close_page(main, window, notification)
            }));
        }
        Ok(Value::Undefined)
    });
    // A channel whose ends can move to other realms of the process.
    let message_channel = realm.function("messageChannel", 0, false, |realm, _, _| {
        let (first, second) = crate::realm_ports::channel(realm);
        Ok(Value::Object(
            realm.array(vec![Value::Object(first), Value::Object(second)]),
        ))
    });
    // Workers: a realm for each, which the page steps after itself.
    let start_worker = realm.function("startWorker", 5, false, |realm, _, args| {
        let id = crate::worker::start(
            realm,
            arg(args, 0),
            arg(args, 1),
            arg(args, 2),
            arg(args, 3),
            arg(args, 4),
        )?;
        Ok(Value::Number(f64::from(id)))
    });
    let terminate_worker = realm.function("terminateWorker", 1, false, |realm, _, args| {
        let id = ops::to_number(realm, arg(args, 0))?;
        crate::worker::terminate(realm, id as u32);
        Ok(Value::Undefined)
    });
    for (name, function) in [
        ("startWorker", start_worker),
        ("terminateWorker", terminate_worker),
        ("messageChannel", message_channel),
        ("notify", notify),
        ("closeNotification", close_notification),
        ("now", now),
        ("locale", locale),
        ("languages", languages),
        ("userAgent", user_agent),
        ("processors", processors),
        ("print", print),
        ("report", report),
        ("title", title),
        ("contextMenu", context_menu),
    ] {
        host.define(Key::name(name), Property::hidden(Value::Object(function)));
    }
    host
}

/// The host object of a worker's prelude: the page's, with what a worker
/// adds. `import(url)` loads the compiled script a URL names, as a worker's
/// script runs; `importScript(url)` runs one before it returns, as
/// `importScripts` does; `close()` ends the worker once its task returns.
pub(crate) fn worker_host(realm: &mut Realm) -> Object {
    let host = host(realm);
    let import = realm.function("import", 1, false, |realm, _, args| {
        realm.dynamic_import_matching(arg(args, 0), &[])
    });
    let import_script = realm.function("importScript", 1, false, |realm, _, args| {
        let url = ops::to_string(realm, arg(args, 0))?.to_rust_lossy();
        let Some(entry) = realm.script_entry(&url) else {
            return Err(realm.throw(
                ErrorKind::Type,
                &format!("Failed to execute 'importScripts': the script at '{url}' was not compiled into this build"),
            ));
        };
        realm.load_entry_now(entry)?;
        Ok(Value::Undefined)
    });
    let close = realm.function("close", 0, false, |realm, _, _| {
        realm.worker_closing = true;
        Ok(Value::Undefined)
    });
    let source_functions = realm.function("sourceFunctions", 0, false, |realm, _, _| {
        Ok(crate::function::source_functions(realm))
    });
    for (name, function) in [
        ("import", import),
        ("importScript", import_script),
        ("close", close),
        ("sourceFunctions", source_functions),
    ] {
        host.define(Key::name(name), Property::hidden(Value::Object(function)));
    }
    host
}

/// Writes `Uncaught` and an error's `name: message` to standard error, as
/// Chromium's console reports an error no script handled.
fn report(realm: &mut Realm, error: &Value) {
    let message = match error {
        Value::Object(_) => crate::builtins::error_to_string(realm, error.clone(), &[])
            .and_then(|text| ops::to_string(realm, &text)),
        value => ops::to_string(realm, value),
    };
    let message = message.map_or_else(|_| "an error".into(), |text| text.to_rust_lossy());
    platform::write_stderr(&format!("Uncaught {message}\n"));
}

/// An exception no script caught in a renderer page: the prelude's
/// `host.onerror` fires the window's `error` event, and an error no listener
/// cancels is reported on the console. The page keeps running.
pub(crate) fn uncaught(realm: &mut Realm, thrown: Thrown) {
    let onerror = match realm.web_host.clone() {
        Some(host) => ops::get_named(realm, &Value::Object(host), "onerror"),
        None => Ok(Value::Undefined),
    };
    let handled = match onerror {
        Ok(onerror @ Value::Object(_)) => ops::call(
            realm,
            &onerror,
            Value::Undefined,
            core::slice::from_ref(&thrown.0),
        )
        .is_ok_and(|handled| ops::to_boolean(&handled)),
        _ => false,
    };
    if !handled {
        report(realm, &thrown.0);
    }
}

/// Whether a value matches an `<input>`'s `pattern` as HTML compiles it:
/// against the whole value, with the `v` flag. `None` when the pattern does
/// not compile.
fn input_pattern_matches(pattern: &str, value: &str) -> Option<bool> {
    let source: Vec<u16> = "^(?:"
        .encode_utf16()
        .chain(pattern.encode_utf16())
        .chain(")$".encode_utf16())
        .collect();
    let flags = nativeboat_regexp::Flags {
        unicode_sets: true,
        ..nativeboat_regexp::Flags::default()
    };
    let compiled = nativeboat_regexp::Pattern::compile(&source, flags).ok()?;
    let text: Vec<u16> = value.encode_utf16().collect();
    Some(compiled.search(&text, 0).is_some())
}

impl Realm {
    /// Gives a renderer page its document, parsed from the page's HTML, and
    /// leaves the prelude's `host`, `native` and `page` objects for the
    /// prelude, which the renderer graph runs first. Returns the document.
    pub fn install_web(&mut self, html: &str, page: Rc<dyn Page>) -> Rc<RefCell<Dom>> {
        let source = nativeboat_html::parse_document(html);
        let mut document = Dom::from_document(&source);
        document.set_pattern_matcher(input_pattern_matches);
        let dom = Rc::new(RefCell::new(document));
        let native = natives(self, &dom, dom_ops::OPERATIONS);
        let page = natives(self, &page, page::OPERATIONS);
        let host = host(self);
        self.web_host = Some(host.clone());
        let arguments = self.array(vec![
            Value::Object(host),
            Value::Object(native),
            Value::Object(page),
        ]);
        self.global.define(
            Key::name(PRELUDE_ARGUMENTS),
            Property::hidden(Value::Object(arguments)),
        );
        dom
    }

    /// Calls a method of the prelude's renderer interface, `host.web`, as the
    /// renderer drives the page: `ready` once scripts ran, then input.
    pub fn web_call(&mut self, method: &str, args: &[Value]) -> Completion {
        let Some(host) = self.web_host.clone() else {
            return Ok(Value::Undefined);
        };
        let web = ops::get(self, &Value::Object(host), &Key::name("web"))?;
        let function = ops::get(self, &web, &Key::name(method))?;
        if !matches!(function, Value::Object(_)) {
            return Ok(Value::Undefined);
        }
        ops::call(self, &function, web, args)
    }
}

#[cfg(test)]
mod tests;
