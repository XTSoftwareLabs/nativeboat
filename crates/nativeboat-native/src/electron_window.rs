//! Electron's event-emitting window objects for the native host: `app`,
//! `BrowserWindow` and `webContents` are Node EventEmitters when the Node.js
//! library is linked, as they are in Electron. Native window messages become
//! their lifecycle events; `webContents.send` queues messages that the host
//! loop delivers to the window's preload `ipcRenderer` listeners.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ffi::c_void;

use crate::app_host::{self, Window, WindowEvent, WindowState};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Completion, Key, Object, Property, Realm, Thrown, Value, ops};

/// `ipcRenderer` listeners of a preload realm: channel, listener, once.
pub(crate) type RendererListeners = Rc<RefCell<Vec<(String, Object, bool)>>>;

/// Node's `EventEmitter` when the Node.js library is linked.
pub(crate) fn event_emitter(realm: &mut Realm) -> Result<Option<Value>, Thrown> {
    if realm.node_library.is_none() {
        return Ok(None);
    }
    let events = crate::modules::load(realm, "node:events")?;
    Ok(crate::array_builtin::is_callable(&events).then_some(events))
}

/// Makes `object` an EventEmitter. With `chain`, that prototype object
/// inherits `EventEmitter.prototype` instead of `object` itself.
pub(crate) fn make_emitter(
    realm: &mut Realm,
    emitter: &Value,
    object: &Object,
    chain: Option<&Object>,
) -> Result<(), Thrown> {
    let prototype = ops::get_named(realm, emitter, "prototype")?;
    chain
        .unwrap_or(object)
        .set_prototype(prototype.as_object().cloned());
    ops::call(realm, emitter, Value::Object(object.clone()), &[])?;
    Ok(())
}

/// `target.emit(name, ...args)`: whether a listener ran. Objects without an
/// `emit` method have no listeners.
pub(crate) fn emit(
    realm: &mut Realm,
    target: &Object,
    name: &str,
    args: &[Value],
) -> Result<bool, Thrown> {
    let target = Value::Object(target.clone());
    let emit = ops::get_named(realm, &target, "emit")?;
    if !crate::array_builtin::is_callable(&emit) {
        return Ok(false);
    }
    let mut arguments = Vec::with_capacity(args.len() + 1);
    arguments.push(Value::str(name));
    arguments.extend_from_slice(args);
    let result = ops::call(realm, &emit, target, &arguments)?;
    Ok(ops::to_boolean(&result))
}

/// Listener methods that accept and ignore listeners, for hosts without the
/// Node.js library.
pub(crate) fn inert_listeners(realm: &Realm, object: &Object) {
    for name in [
        "on",
        "once",
        "addListener",
        "prependListener",
        "off",
        "removeListener",
        "removeAllListeners",
    ] {
        let function = realm.function(name, 2, false, |_, this, _| Ok(this));
        object.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let emit = realm.function("emit", 1, false, |_, _, _| Ok(Value::Boolean(false)));
    object.define(Key::name("emit"), Property::data(Value::Object(emit)));
}

pub(crate) fn define(
    realm: &Realm,
    object: &Object,
    name: &str,
    length: u32,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) {
    let function = realm.function(name, length, false, code);
    object.define(Key::name(name), Property::data(Value::Object(function)));
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

/// An Electron event object whose `preventDefault` records refusal.
fn preventable_event(realm: &Realm) -> (Object, Rc<RefCell<bool>>) {
    let event = realm.object();
    let prevented = Rc::new(RefCell::new(false));
    let flag = prevented.clone();
    define(realm, &event, "preventDefault", 0, move |realm, this, _| {
        *flag.borrow_mut() = true;
        ops::set(
            realm,
            &this,
            &Key::name("defaultPrevented"),
            Value::Boolean(true),
        )?;
        Ok(Value::Undefined)
    });
    event.define(
        Key::name("defaultPrevented"),
        Property::data(Value::Boolean(false)),
    );
    (event, prevented)
}

/// The native window methods MarkText and other Electron applications call
/// on a `BrowserWindow`.
pub(crate) fn install_window_state_methods(realm: &Realm, instance: &Object, window: Window) {
    for (name, state) in [
        ("maximize", WindowState::Maximize),
        ("minimize", WindowState::Minimize),
        ("restore", WindowState::Restore),
        ("unmaximize", WindowState::Restore),
        ("focus", WindowState::Focus),
        ("moveTop", WindowState::Focus),
    ] {
        define(realm, instance, name, 0, move |_, _, _| {
            app_host::set_window_state(window, state);
            Ok(Value::Undefined)
        });
    }
    for (name, index) in [
        ("isMaximized", 0),
        ("isMinimized", 1),
        ("isFocused", 2),
        ("isAlwaysOnTop", 3),
    ] {
        define(realm, instance, name, 0, move |_, _, _| {
            let flags = app_host::window_flags(window);
            Ok(Value::Boolean([flags.0, flags.1, flags.2, flags.3][index]))
        });
    }
    define(realm, instance, "setAlwaysOnTop", 1, move |_, _, args| {
        app_host::set_always_on_top(window, ops::to_boolean(&arg(args, 0)));
        Ok(Value::Undefined)
    });
    define(realm, instance, "setSkipTaskbar", 1, move |_, _, args| {
        app_host::set_skip_taskbar(window, ops::to_boolean(&arg(args, 0)));
        Ok(Value::Undefined)
    });
    define(realm, instance, "setFullScreen", 1, move |_, _, args| {
        app_host::set_window_full_screen(window, ops::to_boolean(&arg(args, 0)));
        Ok(Value::Undefined)
    });
    define(realm, instance, "isFullScreen", 0, move |_, _, _| {
        Ok(Value::Boolean(app_host::is_full_screen(window)))
    });
    define(realm, instance, "setTitle", 1, move |realm, _, args| {
        let title = ops::to_string(realm, &arg(args, 0))?;
        app_host::set_window_title(window, title.units());
        Ok(Value::Undefined)
    });
    define(realm, instance, "getTitle", 0, move |_, _, _| {
        Ok(Value::String(crate::JsStr::from_units(
            &app_host::window_title(window),
        )))
    });
    for (name, content) in [
        ("getBounds", false),
        ("getNormalBounds", false),
        ("getContentBounds", true),
    ] {
        define(realm, instance, name, 0, move |realm, _, _| {
            let bounds = if content {
                app_host::content_bounds(window)
            } else {
                app_host::window_bounds(window)
            };
            let (x, y, width, height) = bounds.unwrap_or_default();
            let bounds = realm.object();
            for (key, value) in [("x", x), ("y", y), ("width", width), ("height", height)] {
                bounds.define(
                    Key::name(key),
                    Property::data(Value::Number(f64::from(value))),
                );
            }
            Ok(Value::Object(bounds))
        });
    }
    for (name, content) in [("getSize", false), ("getContentSize", true)] {
        define(realm, instance, name, 0, move |realm, _, _| {
            let bounds = if content {
                app_host::content_bounds(window)
            } else {
                app_host::window_bounds(window)
            };
            let (_, _, width, height) = bounds.unwrap_or_default();
            Ok(pair(realm, f64::from(width), f64::from(height)))
        });
    }
    define(realm, instance, "setBounds", 1, move |realm, _, args| {
        let bounds = arg(args, 0);
        let (x0, y0, width0, height0) = app_host::window_bounds(window).unwrap_or_default();
        let mut field = |name: &str, current: i32| -> Result<f64, Thrown> {
            Ok(match ops::get(realm, &bounds, &Key::name(name))? {
                Value::Number(value) if value.is_finite() => value,
                _ => f64::from(current),
            })
        };
        let (x, y) = (field("x", x0)?, field("y", y0)?);
        let (width, height) = (field("width", width0)?, field("height", height0)?);
        app_host::set_window_bounds(window, x, y, width, height);
        Ok(Value::Undefined)
    });
    define(realm, instance, "setPosition", 2, move |realm, _, args| {
        let (_, _, width, height) = app_host::window_bounds(window).unwrap_or_default();
        let x = ops::to_number(realm, &arg(args, 0))?;
        let y = ops::to_number(realm, &arg(args, 1))?;
        app_host::set_window_bounds(window, x, y, f64::from(width), f64::from(height));
        Ok(Value::Undefined)
    });
    define(realm, instance, "getPosition", 0, move |realm, _, _| {
        let (x, y, _, _) = app_host::window_bounds(window).unwrap_or_default();
        Ok(Value::Object(realm.array(alloc::vec![
            Value::Number(f64::from(x)),
            Value::Number(f64::from(y)),
        ])))
    });
    define(realm, instance, "setSize", 2, move |realm, _, args| {
        let (x, y, _, _) = app_host::window_bounds(window).unwrap_or_default();
        let width = ops::to_number(realm, &arg(args, 0))?;
        let height = ops::to_number(realm, &arg(args, 1))?;
        app_host::set_window_bounds(window, f64::from(x), f64::from(y), width, height);
        Ok(Value::Undefined)
    });
    define(
        realm,
        instance,
        "setContentSize",
        2,
        move |realm, _, args| {
            let width = ops::to_number(realm, &arg(args, 0))?;
            let height = ops::to_number(realm, &arg(args, 1))?;
            app_host::set_content_size(window, width, height);
            Ok(Value::Undefined)
        },
    );
    for (name, maximum) in [("setMinimumSize", false), ("setMaximumSize", true)] {
        define(realm, instance, name, 2, move |realm, _, args| {
            let size = (
                ops::to_number(realm, &arg(args, 0))?,
                ops::to_number(realm, &arg(args, 1))?,
            );
            let (minimum, current) = app_host::size_limits(window);
            if maximum {
                app_host::set_size_limits(window, minimum, size);
            } else {
                app_host::set_size_limits(window, size, current);
            }
            Ok(Value::Undefined)
        });
    }
    for (name, maximum) in [("getMinimumSize", false), ("getMaximumSize", true)] {
        define(realm, instance, name, 0, move |realm, _, _| {
            let (minimum, limit) = app_host::size_limits(window);
            let (width, height) = if maximum { limit } else { minimum };
            Ok(pair(realm, width, height))
        });
    }
    define(realm, instance, "setResizable", 1, move |_, _, args| {
        app_host::set_resizable(window, ops::to_boolean(&arg(args, 0)));
        Ok(Value::Undefined)
    });
    define(realm, instance, "isResizable", 0, move |_, _, _| {
        Ok(Value::Boolean(app_host::is_resizable(window)))
    });
    define(realm, instance, "center", 0, move |_, _, _| {
        app_host::center(window);
        Ok(Value::Undefined)
    });
    define(realm, instance, "setIcon", 1, move |realm, _, args| {
        let icon = crate::electron_notify::image_icon(realm, &arg(args, 0));
        app_host::set_window_icon(window, icon);
        Ok(Value::Undefined)
    });
    define(
        realm,
        instance,
        "setMenuBarVisibility",
        1,
        move |_, _, args| {
            app_host::set_menu_bar_visibility(window, ops::to_boolean(&arg(args, 0)));
            Ok(Value::Undefined)
        },
    );
    define(realm, instance, "isMenuBarVisible", 0, move |_, _, _| {
        Ok(Value::Boolean(app_host::is_menu_bar_visible(window)))
    });
    define(
        realm,
        instance,
        "setAutoHideMenuBar",
        1,
        move |_, _, args| {
            app_host::set_menu_bar_auto_hide(window, ops::to_boolean(&arg(args, 0)));
            Ok(Value::Undefined)
        },
    );
    define(realm, instance, "isMenuBarAutoHide", 0, move |_, _, _| {
        Ok(Value::Boolean(app_host::is_menu_bar_auto_hide(window)))
    });
    // macOS, taskbar and frame features with no effect on this host.
    for name in [
        "setSheetOffset",
        "setProgressBar",
        "flashFrame",
        "setVibrancy",
        "setRepresentedFilename",
        "setDocumentEdited",
        "blur",
        "setBackgroundColor",
        "setOverlayIcon",
        "setTitleBarOverlay",
        "setAccentColor",
        "setThumbnailClip",
        "setThumbnailToolTip",
        "setAppDetails",
        "setHasShadow",
    ] {
        define(realm, instance, name, 1, |_, _, _| Ok(Value::Undefined));
    }
    // Electron's macOS window states exist on every platform: elsewhere a
    // window is never in simple full screen, has no represented file and is
    // not marked edited.
    define(realm, instance, "setSimpleFullScreen", 1, |_, _, _| {
        Ok(Value::Undefined)
    });
    for name in ["isSimpleFullScreen", "isDocumentEdited"] {
        define(realm, instance, name, 0, |_, _, _| {
            Ok(Value::Boolean(false))
        });
    }
    define(realm, instance, "getRepresentedFilename", 0, |_, _, _| {
        Ok(Value::str(""))
    });
    // Taskbar buttons are not shown, so none is added.
    define(realm, instance, "setThumbarButtons", 1, |_, _, _| {
        Ok(Value::Boolean(false))
    });
    // The window's HWND, in a Buffer of the pointer's bytes as Electron
    // gives it.
    define(
        realm,
        instance,
        "getNativeWindowHandle",
        0,
        move |realm, _, _| {
            let bytes = (window.raw() as u64).to_le_bytes().to_vec();
            let prototype = realm.buffer_prototype.clone();
            Ok(Value::Object(crate::typed_array_builtin::new_bytes(
                realm, bytes, prototype,
            )))
        },
    );
}

/// A `[width, height]` array.
/// The portrait size in millimetres of the paper Electron names.
fn named_paper(name: &str) -> Option<(f32, f32)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "a0" => (841.0, 1189.0),
        "a1" => (594.0, 841.0),
        "a2" => (420.0, 594.0),
        "a3" => (297.0, 420.0),
        "a4" => (210.0, 297.0),
        "a5" => (148.0, 210.0),
        "a6" => (105.0, 148.0),
        "legal" => (215.9, 355.6),
        "letter" => (215.9, 279.4),
        "tabloid" => (279.4, 431.8),
        "ledger" => (431.8, 279.4),
        _ => return None,
    })
}

/// Electron's `print` and `printToPDF` options as a request: `pageSize`
/// by name or in microns, `landscape`, and `margins` in inches.
fn print_request(
    realm: &mut Realm,
    options: &Value,
    target: app_host::PrintTarget,
) -> Result<app_host::PrintRequest, Thrown> {
    let mut request = app_host::PrintRequest {
        target,
        paper: None,
        landscape: false,
        margins: [None; 4],
        name: String::from("Document"),
        background: false,
    };
    if !matches!(options, Value::Object(_)) {
        return Ok(request);
    }
    request.landscape = ops::to_boolean(&ops::get_named(realm, options, "landscape")?);
    request.background = ops::to_boolean(&ops::get_named(realm, options, "printBackground")?);
    let number =
        |realm: &mut Realm, object: &Value, name: &'static str| -> Result<Option<f64>, Thrown> {
            Ok(match ops::get_named(realm, object, name)? {
                Value::Number(value) if value.is_finite() && value >= 0.0 => Some(value),
                _ => None,
            })
        };
    request.paper = match ops::get_named(realm, options, "pageSize")? {
        Value::String(name) => named_paper(&name.to_rust_lossy()),
        size @ Value::Object(_) => {
            match (
                number(realm, &size, "width")?,
                number(realm, &size, "height")?,
            ) {
                (Some(width), Some(height)) if width > 0.0 && height > 0.0 => {
                    Some(((width / 1000.0) as f32, (height / 1000.0) as f32))
                }
                _ => None,
            }
        }
        _ => None,
    };
    let margins = ops::get_named(realm, options, "margins")?;
    if matches!(margins, Value::Object(_)) {
        for (index, side) in ["top", "right", "bottom", "left"].into_iter().enumerate() {
            request.margins[index] =
                number(realm, &margins, side)?.map(|inches| (inches * 96.0) as f32);
        }
    }
    Ok(request)
}

fn pair(realm: &Realm, width: f64, height: f64) -> Value {
    let values = [Value::Number(width), Value::Number(height)];
    Value::Object(realm.array(values.to_vec()))
}

/// A message for a window's renderer: the window, the channel, the
/// arguments, and the message ports moved with it.
pub(crate) type Outgoing = (usize, String, Vec<Value>, Vec<u64>);

/// A window's `webContents`.
pub(crate) fn web_contents(
    realm: &mut Realm,
    emitter: Option<&Value>,
    window: Window,
) -> Result<Object, Thrown> {
    let contents = realm.object();
    match emitter {
        Some(emitter) => make_emitter(realm, emitter, &contents, None)?,
        None => inert_listeners(realm, &contents),
    }
    contents.define(
        Key::name("id"),
        Property::data(Value::Number(window.raw() as f64)),
    );
    define(realm, &contents, "send", 1, move |realm, _, args| {
        let channel = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let values = args.get(1..).unwrap_or_default().to_vec();
        crate::platform::trace_ipc(|| alloc::format!("send {} {channel}", window.raw()));
        realm
            .electron_outgoing
            .push((window.raw(), channel, values, Vec::new()));
        Ok(Value::Undefined)
    });
    // One message with `MessagePortMain`s moved to the renderer, which gets
    // them as the `ports` of its `ipcRenderer` event.
    define(realm, &contents, "postMessage", 2, move |realm, _, args| {
        let channel = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let ports = crate::realm_ports::export_ports(realm, &arg(args, 2))?;
        crate::platform::trace_ipc(|| {
            alloc::format!(
                "postMessage {} {channel} ({} ports)",
                window.raw(),
                ports.len()
            )
        });
        realm
            .electron_outgoing
            .push((window.raw(), channel, vec![arg(args, 1)], ports));
        Ok(Value::Undefined)
    });
    define(realm, &contents, "isDestroyed", 0, move |_, _, _| {
        Ok(Value::Boolean(!app_host::is_window(window)))
    });
    // Zoom is the window's, shared with its page's `webFrame`.
    define(realm, &contents, "getZoomFactor", 0, move |_, _, _| {
        Ok(Value::Number(app_host::zoom(window)))
    });
    define(
        realm,
        &contents,
        "setZoomFactor",
        1,
        move |realm, _, args| {
            let factor = ops::to_number(realm, &arg(args, 0))?;
            if factor.is_nan() || factor <= 0.0 {
                return Err(realm.throw(crate::ErrorKind::Range, "invalid zoom factor"));
            }
            app_host::set_zoom(window, factor);
            Ok(Value::Undefined)
        },
    );
    define(realm, &contents, "getZoomLevel", 0, move |_, _, _| {
        Ok(Value::Number(zoom_level(app_host::zoom(window))))
    });
    define(
        realm,
        &contents,
        "setZoomLevel",
        1,
        move |realm, _, args| {
            let level = ops::to_number(realm, &arg(args, 0))?;
            app_host::set_zoom(window, zoom_factor(level));
            Ok(Value::Undefined)
        },
    );
    for (name, role) in [
        ("undo", "undo"),
        ("redo", "redo"),
        ("cut", "cut"),
        ("copy", "copy"),
        ("paste", "paste"),
        ("pasteAndMatchStyle", "paste"),
        ("delete", "delete"),
        ("selectAll", "selectAll"),
    ] {
        define(realm, &contents, name, 0, move |_, _, _| {
            app_host::send_edit_action(role);
            Ok(Value::Undefined)
        });
    }
    define(realm, &contents, "insertText", 1, move |realm, _, args| {
        // Inserts as typing would, into the page's focused editable text.
        let text = ops::to_string(realm, &arg(args, 0))?;
        app_host::page_text(window, "text", text.units());
        let (promise, resolve, _) = crate::promise_builtin::capability(realm);
        ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
        Ok(promise)
    });
    define(
        realm,
        &contents,
        "replaceMisspelling",
        1,
        move |realm, _, args| {
            let text = ops::to_string(realm, &arg(args, 0))?;
            app_host::page_text(window, "replaceMisspelling", text.units());
            Ok(Value::Undefined)
        },
    );
    define(realm, &contents, "isDevToolsOpened", 0, |_, _, _| {
        Ok(Value::Boolean(false))
    });
    define(realm, &contents, "isLoading", 0, |_, _, _| {
        Ok(Value::Boolean(false))
    });
    define(realm, &contents, "getURL", 0, |_, _, _| Ok(Value::str("")));
    // The user agent the application sets for the window, else the
    // runtime's; pages keep their own `navigator.userAgent`.
    let agent: Rc<RefCell<Option<Value>>> = Rc::default();
    let read = agent.clone();
    define(realm, &contents, "getUserAgent", 0, move |realm, _, _| {
        Ok(read
            .borrow()
            .clone()
            .unwrap_or_else(|| Value::str(&crate::electron_builtin::user_agent(realm))))
    });
    define(
        realm,
        &contents,
        "setUserAgent",
        1,
        move |realm, _, args| {
            let text = ops::to_string(realm, &arg(args, 0))?;
            *agent.borrow_mut() = Some(Value::String(text));
            Ok(Value::Undefined)
        },
    );
    // Pages run in the application's process.
    for name in ["getOSProcessId", "getProcessId"] {
        define(realm, &contents, name, 0, |_, _, _| {
            Ok(Value::Number(f64::from(crate::platform::process_id())))
        });
    }
    define(realm, &contents, "getTitle", 0, move |_, _, _| {
        Ok(Value::String(crate::JsStr::from_units(
            &app_host::window_title(window),
        )))
    });
    define(realm, &contents, "isFocused", 0, move |_, _, _| {
        Ok(Value::Boolean(app_host::window_flags(window).2))
    });
    // Native pages load synchronously, do not crash apart from the
    // application, are painted on screen, and have no developer tools or
    // audio.
    for name in [
        "isLoadingMainFrame",
        "isWaitingForResponse",
        "isCrashed",
        "isOffscreen",
        "isDevToolsFocused",
        "isAudioMuted",
        "isCurrentlyAudible",
        "isBeingCaptured",
    ] {
        define(realm, &contents, name, 0, |_, _, _| {
            Ok(Value::Boolean(false))
        });
    }
    // Searching a page's text is not available: a search finds nothing,
    // so no `found-in-page` event follows, and each gets its request id.
    let requests = Rc::new(core::cell::Cell::new(0u32));
    define(realm, &contents, "findInPage", 2, move |_, _, _| {
        requests.set(requests.get() + 1);
        Ok(Value::Number(f64::from(requests.get())))
    });
    let policy = Rc::new(RefCell::new(String::from("default")));
    let stored = policy.clone();
    define(
        realm,
        &contents,
        "setWebRTCIPHandlingPolicy",
        1,
        move |realm, _, args| {
            *stored.borrow_mut() = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
            Ok(Value::Undefined)
        },
    );
    define(
        realm,
        &contents,
        "getWebRTCIPHandlingPolicy",
        0,
        move |_, _, _| Ok(Value::str(&policy.borrow())),
    );
    define(realm, &contents, "executeJavaScript", 1, |realm, _, _| {
        let (promise, resolve, _) = crate::promise_builtin::capability(realm);
        ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
        Ok(promise)
    });
    define(realm, &contents, "printToPDF", 1, move |realm, _, args| {
        let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
        let options = arg(args, 0);
        let mut request = print_request(realm, &options, app_host::PrintTarget::Pdf)?;
        // Electron prints PDFs on Letter paper unless told otherwise.
        request.paper = request.paper.or(Some((215.9, 279.4)));
        let failure = match app_host::print_window(window, &request) {
            Ok(app_host::PrintOutcome::Pdf(bytes)) => {
                let buffer = crate::electron_clipboard::buffer_value(realm, bytes)?;
                ops::call(realm, &resolve, Value::Undefined, &[buffer])?;
                return Ok(promise);
            }
            Ok(_) => String::from("no PDF was printed"),
            Err(message) => message,
        };
        let error = realm.throw(crate::ErrorKind::Error, &failure);
        ops::call(realm, &reject, Value::Undefined, &[error.0])?;
        Ok(promise)
    });
    define(
        realm,
        &contents,
        "setIgnoreMenuShortcuts",
        1,
        move |_, _, args| {
            app_host::set_ignore_menu_shortcuts(window, ops::to_boolean(&arg(args, 0)));
            Ok(Value::Undefined)
        },
    );
    define(realm, &contents, "print", 2, move |realm, _, args| {
        let options = arg(args, 0);
        let silent = matches!(options, Value::Object(_))
            && ops::to_boolean(&ops::get_named(realm, &options, "silent")?);
        let target = if silent {
            let device = ops::get_named(realm, &options, "deviceName")?;
            let device = match device {
                Value::String(name) => name.to_rust_lossy(),
                _ => String::new(),
            };
            app_host::PrintTarget::Printer(device)
        } else {
            app_host::PrintTarget::Dialog
        };
        let request = print_request(realm, &options, target)?;
        let (success, reason) = match app_host::print_window(window, &request) {
            Ok(app_host::PrintOutcome::Cancelled) => (false, "cancelled"),
            Ok(_) => (true, ""),
            Err(_) => (false, "failed"),
        };
        let callback = arg(args, 1);
        if crate::array_builtin::is_callable(&callback) {
            ops::call(
                realm,
                &callback,
                Value::Undefined,
                &[Value::Boolean(success), Value::str(reason)],
            )?;
        }
        Ok(Value::Undefined)
    });
    // Developer tools, printing dialogs and spelling are not part of the
    // native host.
    for name in [
        "openDevTools",
        "closeDevTools",
        "toggleDevTools",
        "setWindowOpenHandler",
        "reload",
        "focus",
        "showDefinitionForSelection",
        "setVisualZoomLevelLimits",
        "setAudioMuted",
        "setBackgroundThrottling",
        "inspectElement",
        "stopFindInPage",
        "beginFrameSubscription",
        "endFrameSubscription",
        "copyImageAt",
        "startDrag",
        "invalidate",
        "setFrameRate",
        "reloadIgnoringCache",
    ] {
        define(realm, &contents, name, 1, |_, _, _| Ok(Value::Undefined));
    }
    // Every window's contents share the default session, as in Electron;
    // a realm without the Electron module gets one of its own.
    let session = match realm.electron_session.clone() {
        Some(session) => session,
        None => {
            let session = realm.object();
            match emitter {
                Some(emitter) => make_emitter(realm, emitter, &session, None)?,
                None => inert_listeners(realm, &session),
            }
            install_spell_checker(realm, &session);
            crate::electron_builtin::session_handlers(realm, &session);
            session
        }
    };
    contents.define(Key::name("session"), Property::data(Value::Object(session)));
    Ok(contents)
}

/// Chromium's zoom level of a zoom factor: each level zooms 20% more.
pub(crate) fn zoom_level(factor: f64) -> f64 {
    crate::libm::log(factor) / crate::libm::log(1.2)
}

/// The zoom factor of a Chromium zoom level.
pub(crate) fn zoom_factor(level: f64) -> f64 {
    crate::libm::pow(1.2, level)
}

/// A session's spell checker: the Windows spell checker, shared by the
/// application's windows as Electron's default session is.
pub(crate) fn install_spell_checker(realm: &Realm, session: &Object) {
    let strings = |realm: &mut Realm, list: Vec<String>| {
        let values = list.iter().map(|text| Value::str(text)).collect();
        Value::Object(realm.array(values))
    };
    let available = realm.function(
        "availableSpellCheckerLanguages",
        0,
        false,
        move |realm, _, _| Ok(strings(realm, crate::spelling::available_languages())),
    );
    session.define(
        Key::name("availableSpellCheckerLanguages"),
        Property {
            slot: crate::Slot::Accessor {
                get: Some(available),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    define(
        realm,
        session,
        "getSpellCheckerLanguages",
        0,
        move |realm, _, _| {
            // Without languages set, the application's locale is checked.
            let mut languages = crate::spelling::languages();
            if languages.is_empty() {
                languages.push(crate::platform::user_locale());
            }
            Ok(strings(realm, languages))
        },
    );
    define(
        realm,
        session,
        "setSpellCheckerLanguages",
        1,
        |realm, _, args| {
            let list = arg(args, 0);
            let length = ops::get_named(realm, &list, "length")?;
            let length = crate::array_builtin::to_length(realm, &length)?;
            let mut languages = Vec::new();
            for index in 0..length {
                let language = ops::get(realm, &list, &Key::Index(index as u32))?;
                languages.push(ops::to_string(realm, &language)?.to_rust_lossy());
            }
            crate::spelling::set_languages(&languages);
            Ok(Value::Undefined)
        },
    );
    define(realm, session, "isSpellCheckerEnabled", 0, |_, _, _| {
        Ok(Value::Boolean(crate::spelling::enabled()))
    });
    define(realm, session, "setSpellCheckerEnabled", 1, |_, _, args| {
        crate::spelling::set_enabled(ops::to_boolean(&arg(args, 0)));
        Ok(Value::Undefined)
    });
    define(
        realm,
        session,
        "addWordToSpellCheckerDictionary",
        1,
        |realm, _, args| {
            let word = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
            Ok(Value::Boolean(crate::spelling::add_word(&word)))
        },
    );
    define(
        realm,
        session,
        "removeWordFromSpellCheckerDictionary",
        1,
        |realm, _, args| {
            let word = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
            Ok(Value::Boolean(crate::spelling::remove_word(&word)))
        },
    );
    define(
        realm,
        session,
        "listWordsInSpellCheckerDictionary",
        0,
        move |realm, _, _| {
            let (promise, resolve, _) = crate::promise_builtin::capability(realm);
            let words = strings(realm, crate::spelling::words());
            ops::call(realm, &resolve, Value::Undefined, &[words])?;
            Ok(promise)
        },
    );
}

impl Realm {
    /// Records a main-realm window, its `BrowserWindow` and `webContents`.
    pub(crate) fn register_electron_window(
        &mut self,
        window: usize,
        instance: Object,
        contents: Object,
    ) {
        self.electron_windows.push((window, instance, contents));
    }

    pub(crate) fn electron_window_objects(&self, window: usize) -> Option<(Object, Object)> {
        self.electron_windows
            .iter()
            .find(|(known, _, _)| *known == window)
            .map(|(_, instance, contents)| (instance.clone(), contents.clone()))
    }

    /// The `webContents` of a main-realm window, as IPC events' `sender`.
    pub(crate) fn electron_web_contents(&self, window: usize) -> Option<Object> {
        self.electron_window_objects(window)
            .map(|(_, contents)| contents)
    }

    /// Marks this realm as running inside a window's page, for IPC senders.
    pub fn set_electron_window(&mut self, window: usize) {
        self.electron_window = window;
    }

    /// Reports that a window's page has initialized: `dom-ready` and
    /// `did-finish-load` on its `webContents`, then `ready-to-show` on the
    /// window. Listener errors are uncaught exceptions of this realm.
    pub fn electron_page_loaded(&mut self, window: usize) {
        crate::platform::trace_ipc(|| alloc::format!("loaded {window}"));
        let Some((instance, contents)) = self.electron_window_objects(window) else {
            return;
        };
        for (target, name) in [
            (&contents, "dom-ready"),
            (&contents, "did-finish-load"),
            (&instance, "ready-to-show"),
        ] {
            let event = Value::Object(self.object());
            if let Err(thrown) = emit(self, target, name, &[event]) {
                self.host_errors.push(thrown);
            }
        }
        // Then the window's `loadFile` and `loadURL` promises settle.
        let (loaded, waiting): (Vec<_>, Vec<_>) = core::mem::take(&mut self.electron_load_waiters)
            .into_iter()
            .partition(|(known, _)| *known == window);
        self.electron_load_waiters = waiting;
        for (_, resolve) in loaded {
            if let Err(thrown) = ops::call(self, &resolve, Value::Undefined, &[Value::Undefined]) {
                self.host_errors.push(thrown);
            }
        }
    }

    /// Whether `webContents.send` messages for a window wait to be
    /// delivered.
    pub fn has_electron_messages(&self, window: usize) -> bool {
        self.electron_outgoing
            .iter()
            .any(|(target, ..)| *target == window)
            || self
                .page_notification_events
                .iter()
                .any(|(target, _, _)| *target == window)
    }

    /// Delivers this main realm's queued `webContents.send` messages for a
    /// window to its preload realm's `ipcRenderer` listeners. The page's
    /// realm gets them next, from [`Realm::deliver_electron_messages_to_page`].
    pub fn deliver_electron_messages(&mut self, window: usize, preload: &mut Realm) {
        let (mut messages, kept): (Vec<_>, Vec<_>) = core::mem::take(&mut self.electron_outgoing)
            .into_iter()
            .partition(|(target, ..)| *target == window);
        self.electron_outgoing = kept;
        self.deliver_to(window, preload, &mut messages);
        self.electron_preloaded.extend(messages);
    }

    /// Delivers the messages for a window, its preload's included, to the
    /// `ipcRenderer` listeners of its page's realm, as a page with Node
    /// integration registers them through `require('electron')`.
    pub fn deliver_electron_messages_to_page(&mut self, window: usize, page: &mut Realm) {
        let mine = |(target, ..): &Outgoing| *target == window;
        let (preloaded, kept): (Vec<_>, Vec<_>) = core::mem::take(&mut self.electron_preloaded)
            .into_iter()
            .partition(mine);
        self.electron_preloaded = kept;
        let (direct, kept): (Vec<_>, Vec<_>) = core::mem::take(&mut self.electron_outgoing)
            .into_iter()
            .partition(mine);
        self.electron_outgoing = kept;
        let mut messages: Vec<_> = preloaded.into_iter().chain(direct).collect();
        self.deliver_to(window, page, &mut messages);
        let (events, kept): (Vec<_>, Vec<_>) = core::mem::take(&mut self.page_notification_events)
            .into_iter()
            .partition(|(target, _, _)| *target == window);
        self.page_notification_events = kept;
        #[cfg(feature = "dom")]
        for (_, notification, kind) in events {
            let args = [Value::Number(f64::from(notification)), Value::str(kind)];
            if let Err(thrown) = page.web_call("notification", &args) {
                page.host_errors.push(thrown);
            }
        }
        #[cfg(not(feature = "dom"))]
        drop(events);
    }

    fn deliver_to(&mut self, window: usize, preload: &mut Realm, messages: &mut [Outgoing]) {
        let Some(listeners) = preload.electron_renderer_listeners.clone() else {
            return;
        };
        for (_, channel, values, ports) in messages.iter_mut() {
            let (channel, values) = (&*channel, &*values);
            let mut arguments = Vec::with_capacity(values.len() + 1);
            let event = preload.object();
            event.define(Key::name("senderId"), Property::data(Value::Number(0.0)));
            arguments.push(Value::Object(event.clone()));
            let mut cloned = true;
            for value in values {
                match crate::structured_clone_builtin::clone_between(self, preload, value) {
                    Ok(value) => arguments.push(value),
                    Err(_) => cloned = false,
                }
            }
            if !cloned {
                crate::platform::trace_ipc(|| {
                    alloc::format!("drop {window} {channel}: arguments could not be cloned")
                });
                continue;
            }
            let matching: Vec<(Object, bool)> = listeners
                .borrow()
                .iter()
                .filter(|(known, _, _)| known == channel)
                .map(|(_, listener, once)| (listener.clone(), *once))
                .collect();
            crate::platform::trace_ipc(|| {
                alloc::format!("deliver {window} {channel} to {} listeners", matching.len())
            });
            listeners
                .borrow_mut()
                .retain(|(known, _, once)| !(known == channel && *once));
            // The first realm listening on the channel gets the ports.
            let moved = if matching.is_empty() {
                Vec::new()
            } else {
                core::mem::take(ports)
            };
            let moved = crate::realm_ports::adopt(preload, &moved);
            event.define(
                Key::name("ports"),
                Property::data(Value::Object(preload.array(moved))),
            );
            for (listener, _) in matching {
                if let Err(thrown) = ops::call(
                    preload,
                    &Value::Object(listener),
                    Value::Undefined,
                    &arguments,
                ) {
                    preload.host_errors.push(thrown);
                }
            }
        }
    }
}

/// A key as Electron's `before-input-event` reports it.
#[derive(Debug, Clone, Default)]
pub struct KeyInput<'a> {
    /// Pressed rather than released.
    pub down: bool,
    /// The DOM `key`.
    pub key: &'a str,
    /// The DOM `code`.
    pub code: &'a str,
    /// A repeated press.
    pub repeat: bool,
    /// Part of an input method composition.
    pub composing: bool,
    /// Held Shift.
    pub shift: bool,
    /// Held Control.
    pub control: bool,
    /// Held Alt.
    pub alt: bool,
    /// Held Windows key.
    pub meta: bool,
    /// The DOM key location: standard, left, right or numeric keypad.
    pub location: u8,
    /// Caps Lock is on.
    pub caps_lock: bool,
    /// Num Lock is on.
    pub num_lock: bool,
}

impl Realm {
    /// Emits `before-input-event` for a key of a main-realm window on its
    /// `webContents`; returns whether a listener prevented the page from
    /// receiving the key. Listener errors are uncaught exceptions.
    pub fn electron_before_input(&mut self, window: usize, input: &KeyInput) -> bool {
        let Some((_, contents)) = self.electron_window_objects(window) else {
            return false;
        };
        let (event, prevented) = preventable_event(self);
        let details = self.object();
        let modifiers: Vec<Value> = [
            ("shift", input.shift),
            ("control", input.control),
            ("alt", input.alt),
            ("meta", input.meta),
            ("isautorepeat", input.repeat),
            ("iskeypad", input.location == 3),
            ("left", input.location == 1),
            ("right", input.location == 2),
            ("capslock", input.caps_lock),
            ("numlock", input.num_lock),
        ]
        .into_iter()
        .filter(|(_, held)| *held)
        .map(|(name, _)| Value::str(name))
        .collect();
        let modifiers = Value::Object(self.array(modifiers));
        for (name, value) in [
            (
                "type",
                Value::str(if input.down { "keyDown" } else { "keyUp" }),
            ),
            ("key", Value::str(input.key)),
            ("code", Value::str(input.code)),
            ("isAutoRepeat", Value::Boolean(input.repeat)),
            ("isComposing", Value::Boolean(input.composing)),
            ("shift", Value::Boolean(input.shift)),
            ("control", Value::Boolean(input.control)),
            ("alt", Value::Boolean(input.alt)),
            ("meta", Value::Boolean(input.meta)),
            ("location", Value::Number(f64::from(input.location))),
            ("modifiers", modifiers),
        ] {
            details.define(Key::name(name), Property::data(value));
        }
        let args = [Value::Object(event), Value::Object(details)];
        if let Err(thrown) = emit(self, &contents, "before-input-event", &args) {
            self.host_errors.push(thrown);
        }
        *prevented.borrow()
    }
}

/// The host's window event callback; `context` is the main realm.
///
/// # Safety
/// `context` points to the live main `Realm` for the whole message loop, and
/// is only used on the UI thread.
pub unsafe extern "system" fn window_event(
    context: *mut c_void,
    window: usize,
    event: WindowEvent,
) -> bool {
    if context.is_null() {
        return true;
    }
    // SAFETY: guaranteed by the caller.
    let realm = unsafe { &mut *context.cast::<Realm>() };
    let Some((instance, contents)) = realm.electron_window_objects(window) else {
        return true;
    };
    if matches!(event, WindowEvent::ZoomIn | WindowEvent::ZoomOut) {
        let (event_object, _) = preventable_event(realm);
        let direction = Value::str(if event == WindowEvent::ZoomIn {
            "in"
        } else {
            "out"
        });
        let arguments = [Value::Object(event_object), direction];
        if let Err(thrown) = emit(realm, &contents, "zoom-changed", &arguments) {
            realm.host_errors.push(thrown);
        }
        return true;
    }
    let name = match event {
        WindowEvent::Close => "close",
        WindowEvent::Closed => "closed",
        WindowEvent::Focus => "focus",
        WindowEvent::Blur => "blur",
        WindowEvent::Resize => "resize",
        WindowEvent::Maximize => "maximize",
        WindowEvent::Unmaximize => "unmaximize",
        WindowEvent::Minimize => "minimize",
        WindowEvent::Restore => "restore",
        WindowEvent::EnterFullScreen => "enter-full-screen",
        WindowEvent::LeaveFullScreen => "leave-full-screen",
        WindowEvent::ZoomIn | WindowEvent::ZoomOut => return true,
    };
    let (event_object, prevented) = preventable_event(realm);
    if let Err(thrown) = emit(realm, &instance, name, &[Value::Object(event_object)]) {
        realm.host_errors.push(thrown);
    }
    if event == WindowEvent::Closed {
        realm
            .electron_windows
            .retain(|(known, _, _)| *known != window);
    }
    let refused = event == WindowEvent::Close && *prevented.borrow();
    if refused {
        // A window that refuses to close cancels a quit in progress.
        realm.electron_quitting = false;
    }
    if event == WindowEvent::Closed && realm.electron_windows.is_empty() {
        let result = if realm.electron_quitting {
            finish_quit(realm)
        } else {
            match realm.electron_app.clone() {
                // Without listeners the app quits, as Electron's default does.
                Some(app) => match emit(realm, &app, "window-all-closed", &[]) {
                    Ok(true) => Ok(()),
                    Ok(false) => quit(realm),
                    Err(thrown) => Err(thrown),
                },
                None => Ok(()),
            }
        };
        if let Err(thrown) = result {
            realm.host_errors.push(thrown);
        }
    }
    !refused
}

/// A window's page changed its title: `page-title-updated` on its web
/// contents and window, and unless a listener prevents it, the window takes
/// the title.
#[cfg(feature = "dom")]
pub(crate) fn page_title_updated(
    realm: &mut Realm,
    window: usize,
    title: &crate::JsStr,
) -> Result<(), Thrown> {
    let Some((instance, contents)) = realm.electron_window_objects(window) else {
        return Ok(());
    };
    let (event, prevented) = preventable_event(realm);
    let args = [
        Value::Object(event),
        Value::String(title.clone()),
        Value::Boolean(true),
    ];
    emit(realm, &contents, "page-title-updated", &args)?;
    emit(realm, &instance, "page-title-updated", &args)?;
    if !*prevented.borrow() {
        app_host::set_window_title(app_host::Window::from_raw(window), title.units());
    }
    Ok(())
}

/// A window's page asked for a context menu: `context-menu` on its
/// `webContents` with the parameters the page describes.
#[cfg(feature = "dom")]
pub(crate) fn context_menu(realm: &mut Realm, window: usize, params: Value) -> Result<(), Thrown> {
    let Some((_, contents)) = realm.electron_window_objects(window) else {
        return Ok(());
    };
    let (event, _) = preventable_event(realm);
    emit(
        realm,
        &contents,
        "context-menu",
        &[Value::Object(event), params],
    )?;
    Ok(())
}

/// `app.quit()`: emits `before-quit`, then asks every window to close. Once
/// the last one has closed the app emits `will-quit` and `quit` and ends. A
/// listener that prevents `before-quit` or `will-quit`, or a window that
/// refuses to close, cancels the quit.
pub(crate) fn quit(realm: &mut Realm) -> Result<(), Thrown> {
    if realm.electron_quitting {
        return Ok(());
    }
    if let Some(app) = realm.electron_app.clone() {
        let (event, prevented) = preventable_event(realm);
        emit(realm, &app, "before-quit", &[Value::Object(event)])?;
        if *prevented.borrow() {
            return Ok(());
        }
    }
    realm.electron_quitting = true;
    let windows: Vec<usize> = realm
        .electron_windows
        .iter()
        .map(|(window, _, _)| *window)
        .collect();
    if windows.is_empty() {
        return finish_quit(realm);
    }
    for window in windows {
        let _ = app_host::request_close(app_host::Window::from_raw(window));
    }
    Ok(())
}

/// The end of a quit once every window has closed.
fn finish_quit(realm: &mut Realm) -> Result<(), Thrown> {
    if let Some(app) = realm.electron_app.clone() {
        let (event, prevented) = preventable_event(realm);
        emit(realm, &app, "will-quit", &[Value::Object(event)])?;
        if *prevented.borrow() {
            realm.electron_quitting = false;
            return Ok(());
        }
        let event = Value::Object(realm.object());
        emit(realm, &app, "quit", &[event, Value::Number(0.0)])?;
    }
    app_host::request_quit(0);
    Ok(())
}

#[cfg(test)]
#[path = "electron_window/tests.rs"]
mod tests;
