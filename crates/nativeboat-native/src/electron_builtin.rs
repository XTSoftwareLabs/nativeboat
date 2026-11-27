//! The first engine-free Electron host surface: app readiness and native
//! top-level windows. Renderer loading and the rest of Electron remain separate
//! integration work.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::prelude::*;
use crate::{Key, Object, Property, Realm, Thrown, Value, app_host, ops};

/// Builds the native Electron module exports supported by this host slice.
pub fn module(realm: &mut Realm) -> Object {
    if let Some(exports) = realm.electron_exports.clone() {
        return exports;
    }
    let exports = realm.object();
    realm.electron_exports = Some(exports.clone());
    // Electron's app, windows and contents are Node EventEmitters.
    let emitter = crate::electron_window::event_emitter(realm).ok().flatten();
    let app = app(realm);
    match &emitter {
        Some(emitter) => {
            if crate::electron_window::make_emitter(realm, emitter, &app, None).is_ok() {
                // `ready` follows the main script, as the app becomes ready.
                let ready = app.clone();
                realm.enqueue(Box::new(move |realm| {
                    let launch = Value::Object(realm.object());
                    let event = Value::Object(realm.object());
                    crate::electron_window::emit(realm, &ready, "ready", &[event, launch]).map(drop)
                }));
            }
        }
        None => install_ready_listener(realm, &app),
    }
    realm.electron_app = Some(app.clone());
    exports.define(Key::name("app"), Property::data(Value::Object(app)));
    let window = browser_window_constructor(realm, emitter.clone());
    exports.define(
        Key::name("BrowserWindow"),
        Property::data(Value::Object(window)),
    );
    exports.define(
        Key::name("ipcMain"),
        Property::data(Value::Object(crate::electron_ipc::module(realm))),
    );
    exports.define(
        Key::name("contextBridge"),
        Property::data(Value::Object(context_bridge(realm))),
    );
    if realm.electron_ipc_main != 0 {
        exports.define(
            Key::name("ipcRenderer"),
            Property::data(Value::Object(ipc_renderer(realm))),
        );
    }
    exports.define(
        Key::name("webFrame"),
        Property::data(Value::Object(web_frame(realm))),
    );
    exports.define(
        Key::name("webUtils"),
        Property::data(Value::Object(web_utils(realm))),
    );
    exports.define(Key::name("net"), Property::data(Value::Object(net(realm))));
    exports.define(
        Key::name("webContents"),
        Property::data(Value::Object(web_contents_module(realm))),
    );
    // The default session's protocol is the module's: one registry of the
    // schemes the main process serves.
    let protocol = crate::electron_protocol::protocol(realm);
    let (session, default_session) = session(realm, &protocol);
    match &emitter {
        Some(emitter) => {
            let _ = crate::electron_window::make_emitter(realm, emitter, &default_session, None);
        }
        None => crate::electron_window::inert_listeners(realm, &default_session),
    }
    crate::electron_window::install_spell_checker(realm, &default_session);
    realm.electron_session = Some(default_session);
    exports.define(Key::name("session"), Property::data(Value::Object(session)));
    exports.define(
        Key::name("clipboard"),
        Property::data(Value::Object(crate::electron_clipboard::module(realm))),
    );
    exports.define(
        Key::name("dialog"),
        Property::data(Value::Object(crate::electron_dialog::module(realm))),
    );
    let menu = crate::electron_menu::module(realm);
    for name in ["Menu", "MenuItem"] {
        if let Some(crate::Property {
            slot:
                crate::Slot::Data {
                    value: Value::Object(export),
                    ..
                },
            ..
        }) = menu.own_property(&Key::name(name))
        {
            exports.define(Key::name(name), Property::data(Value::Object(export)));
        }
    }
    exports.define(
        Key::name("shell"),
        Property::data(Value::Object(crate::electron_shell::module(realm))),
    );
    // `screen` is an EventEmitter, as in Electron. Display changes are not
    // reported yet, so its listeners are kept without being called.
    let screen = crate::electron_screen::module(realm);
    match &emitter {
        Some(emitter) => {
            let _ = crate::electron_window::make_emitter(realm, emitter, &screen, None);
        }
        None => crate::electron_window::inert_listeners(realm, &screen),
    }
    exports.define(Key::name("screen"), Property::data(Value::Object(screen)));
    exports.define(
        Key::name("nativeTheme"),
        Property::data(Value::Object(native_theme(realm))),
    );
    exports.define(
        Key::name("crashReporter"),
        Property::data(Value::Object(crate::electron_crash_reporter::module(realm))),
    );
    exports.define(
        Key::name("powerSaveBlocker"),
        Property::data(Value::Object(crate::electron_power::module(realm))),
    );
    // Made when first used: they listen to the system from then on.
    for (name, make) in [
        (
            "powerMonitor",
            crate::electron_system::power_monitor as fn(&mut Realm, Option<&Value>) -> Object,
        ),
        (
            "systemPreferences",
            crate::electron_system::system_preferences,
        ),
    ] {
        let made: Rc<RefCell<Option<Object>>> = Rc::default();
        let emitter = emitter.clone();
        let getter = realm.function(
            &alloc::format!("get {name}"),
            0,
            false,
            move |realm, _, _| {
                if let Some(module) = made.borrow().as_ref() {
                    return Ok(Value::Object(module.clone()));
                }
                let module = make(realm, emitter.as_ref());
                *made.borrow_mut() = Some(module.clone());
                Ok(Value::Object(module))
            },
        );
        ops::define_accessor(&exports, Key::name(name), Some(getter), None);
    }
    // Modules the Node library implements, which the compiler links when a
    // program imports them: child processes and their message ports, and
    // screen sources and tracing.
    for (name, library) in [
        ("utilityProcess", "node:internal/electron/utility_process"),
        (
            "MessageChannelMain",
            "node:internal/electron/utility_process",
        ),
        ("contentTracing", "node:internal/electron/media"),
        ("desktopCapturer", "node:internal/electron/media"),
    ] {
        let getter = realm.function(
            &alloc::format!("get {name}"),
            0,
            false,
            move |realm, _, _| {
                // A program that does not import it does not link it: the
                // export is undefined, as reading every export shows it.
                if realm.node_library.is_none() {
                    return Ok(Value::Undefined);
                }
                let module = crate::modules::load(realm, library)?;
                if matches!(module, Value::Undefined) {
                    return Ok(Value::Undefined);
                }
                ops::get_named(realm, &module, name)
            },
        );
        ops::define_accessor(&exports, Key::name(name), Some(getter), None);
    }
    // Classes a native application cannot provide: programs that import
    // them compile, with a warning, and constructing one throws.
    let view = realm.function("WebContentsView", 1, true, |realm, _, _| {
        Err(realm.throw(
            crate::ErrorKind::Error,
            "WebContentsView is not available in native applications: web content views need Chromium's compositor",
        ))
    });
    exports.define(
        Key::name("WebContentsView"),
        Property::data(Value::Object(view)),
    );
    exports.define(
        Key::name("safeStorage"),
        Property::data(Value::Object(crate::electron_safe_storage::module(realm))),
    );
    exports.define(
        Key::name("nativeImage"),
        Property::data(Value::Object(crate::electron_native_image::module(realm))),
    );
    let tray = crate::electron_notify::tray(realm, emitter.clone());
    exports.define(Key::name("Tray"), Property::data(Value::Object(tray)));
    let notification = crate::electron_notify::notification(realm, emitter.clone());
    exports.define(
        Key::name("Notification"),
        Property::data(Value::Object(notification)),
    );
    let global_shortcut = crate::electron_notify::global_shortcut(realm);
    exports.define(
        Key::name("globalShortcut"),
        Property::data(Value::Object(global_shortcut)),
    );
    exports.define(
        Key::name("protocol"),
        Property::data(Value::Object(protocol)),
    );
    let auto_updater = auto_updater(realm, emitter.as_ref());
    exports.define(
        Key::name("autoUpdater"),
        Property::data(Value::Object(auto_updater)),
    );
    exports
}

/// Electron's `webContents` module: the contents of the application's live
/// windows. Frames and DevTools targets are not native objects, so looking
/// contents up by them finds none.
fn web_contents_module(realm: &Realm) -> Object {
    let module = realm.object();
    let all = realm.function("getAllWebContents", 0, false, |realm, _, _| {
        let contents = realm
            .electron_windows
            .iter()
            .filter(|(window, _, _)| app_host::is_window(app_host::Window::from_raw(*window)))
            .map(|(_, _, contents)| Value::Object(contents.clone()))
            .collect();
        Ok(Value::Object(realm.array(contents)))
    });
    let from_id = realm.function("fromId", 1, false, |realm, _, args| {
        let id = ops::to_number(realm, args.first().unwrap_or(&Value::Undefined))?;
        let found = realm
            .electron_windows
            .iter()
            .find(|(window, _, _)| {
                *window as f64 == id && app_host::is_window(app_host::Window::from_raw(*window))
            })
            .map(|(_, _, contents)| contents.clone());
        Ok(found.map_or(Value::Undefined, Value::Object))
    });
    let focused = realm.function("getFocusedWebContents", 0, false, |realm, _, _| {
        let found = realm
            .electron_windows
            .iter()
            .find(|(window, _, _)| app_host::window_flags(app_host::Window::from_raw(*window)).2)
            .map(|(_, _, contents)| contents.clone());
        Ok(found.map_or(Value::Null, Value::Object))
    });
    let from_frame = realm.function("fromFrame", 1, false, |_, _, _| Ok(Value::Undefined));
    let from_target = realm.function("fromDevToolsTargetId", 1, false, |_, _, _| {
        Ok(Value::Undefined)
    });
    for (name, function) in [
        ("getAllWebContents", all),
        ("fromId", from_id),
        ("getFocusedWebContents", focused),
        ("fromFrame", from_frame),
        ("fromDevToolsTargetId", from_target),
    ] {
        module.define(Key::name(name), Property::data(Value::Object(function)));
    }
    module
}

/// Electron's Squirrel.Windows `autoUpdater` as it behaves in applications
/// Squirrel did not install, which native builds are: it keeps its feed
/// URL, checks report that the URL is not set or that Squirrel is missing,
/// and there is never an update to install. Its events work, so updaters
/// such as electron-updater can announce `before-quit-for-update` on it.
fn auto_updater(realm: &mut Realm, emitter: Option<&Value>) -> Object {
    let updater = realm.object();
    match emitter {
        Some(emitter) => {
            let _ = crate::electron_window::make_emitter(realm, emitter, &updater, None);
        }
        None => crate::electron_window::inert_listeners(realm, &updater),
    }
    let feed = Rc::new(RefCell::new(String::new()));
    let stored = feed.clone();
    let set_feed = realm.function("setFeedURL", 1, false, move |realm, _, args| {
        let options = args.first().cloned().unwrap_or(Value::Undefined);
        let url = match &options {
            Value::Object(_) => ops::get_named(realm, &options, "url")?,
            _ => options,
        };
        if !matches!(url, Value::String(_)) {
            return Err(realm.throw(
                crate::ErrorKind::Type,
                "Expected options object to contain a 'url' string property in setFeedUrl call",
            ));
        }
        *stored.borrow_mut() = ops::to_string(realm, &url)?.to_rust_lossy();
        Ok(Value::Undefined)
    });
    updater.define(
        Key::name("setFeedURL"),
        Property::data(Value::Object(set_feed)),
    );
    let read = feed.clone();
    let get_feed = realm.function("getFeedURL", 0, false, move |_, _, _| {
        Ok(Value::str(&read.borrow()))
    });
    updater.define(
        Key::name("getFeedURL"),
        Property::data(Value::Object(get_feed)),
    );
    for (name, length) in [("checkForUpdates", 0), ("quitAndInstall", 0)] {
        let feed = feed.clone();
        let target = updater.clone();
        let method = realm.function(name, length, false, move |realm, _, _| {
            let message = match (name, feed.borrow().is_empty()) {
                ("quitAndInstall", _) => "No update available, can't quit and install",
                (_, true) => "Update URL is not set",
                (_, false) => "Can not find Squirrel",
            };
            let error = realm.error(crate::ErrorKind::Error, message);
            crate::electron_window::emit(realm, &target, "error", &[Value::Object(error)])?;
            Ok(Value::Undefined)
        });
        updater.define(Key::name(name), Property::data(Value::Object(method)));
    }
    updater
}

fn context_bridge(realm: &Realm) -> Object {
    let bridge = realm.object();
    let expose = realm.function("exposeInMainWorld", 2, false, |realm, _, args| {
        let key = args.first().cloned().unwrap_or(Value::Undefined);
        let key = ops::to_string(realm, &key)?.to_rust_lossy();
        let value = args.get(1).cloned().unwrap_or(Value::Undefined);
        if let Some((_, current)) = realm
            .electron_bridge_exports
            .iter_mut()
            .find(|(name, _)| *name == key)
        {
            *current = value.clone();
        } else {
            realm
                .electron_bridge_exports
                .push((key.clone(), value.clone()));
        }
        realm.set_global(&key, value);
        Ok(Value::Undefined)
    });
    bridge.define(
        Key::name("exposeInMainWorld"),
        Property::data(Value::Object(expose)),
    );
    let expose_world = realm.function("exposeInIsolatedWorld", 3, false, |realm, _, args| {
        let key = args.get(1).cloned().unwrap_or(Value::Undefined);
        let value = args.get(2).cloned().unwrap_or(Value::Undefined);
        let expose = context_bridge(realm);
        let method = ops::get(
            realm,
            &Value::Object(expose),
            &Key::name("exposeInMainWorld"),
        )?;
        ops::call(realm, &method, Value::Undefined, &[key, value])
    });
    bridge.define(
        Key::name("exposeInIsolatedWorld"),
        Property::data(Value::Object(expose_world)),
    );
    bridge
}

fn ipc_renderer(realm: &mut Realm) -> Object {
    let renderer = realm.object();
    let listeners = Rc::new(RefCell::new(Vec::<(String, Object, bool)>::new()));
    // Messages from the main realm's `webContents.send` reach these.
    realm.electron_renderer_listeners = Some(listeners.clone());
    for (name, once) in [("on", false), ("addListener", false), ("once", true)] {
        let listeners = listeners.clone();
        renderer.define(
            Key::name(name),
            Property::data(Value::Object(realm.function(
                name,
                2,
                false,
                move |realm, this, args| {
                    let channel = ipc_channel(realm, args)?;
                    let Some(Value::Object(listener)) = args.get(1) else {
                        return Err(
                            realm.throw(crate::ErrorKind::Type, "listener must be a function")
                        );
                    };
                    if !crate::array_builtin::is_callable(&Value::Object(listener.clone())) {
                        return Err(
                            realm.throw(crate::ErrorKind::Type, "listener must be a function")
                        );
                    }
                    crate::platform::trace_ipc(|| alloc::format!("listen {channel}"));
                    listeners
                        .borrow_mut()
                        .push((channel, listener.clone(), once));
                    Ok(this)
                },
            ))),
        );
    }
    for name in ["removeListener", "off"] {
        let listeners = listeners.clone();
        renderer.define(
            Key::name(name),
            Property::data(Value::Object(realm.function(
                name,
                2,
                false,
                move |realm, this, args| {
                    let channel = ipc_channel(realm, args)?;
                    if let Some(Value::Object(listener)) = args.get(1) {
                        listeners.borrow_mut().retain(|(known, callback, _)| {
                            known != &channel
                                || !crate::same_value(
                                    &Value::Object(callback.clone()),
                                    &Value::Object(listener.clone()),
                                )
                        });
                    }
                    Ok(this)
                },
            ))),
        );
    }
    let listeners_for_clear = listeners.clone();
    renderer.define(
        Key::name("removeAllListeners"),
        Property::data(Value::Object(realm.function(
            "removeAllListeners",
            1,
            false,
            move |realm, this, args| {
                if let Some(channel) = args.first() {
                    let channel = ops::to_string(realm, channel)?.to_rust_lossy();
                    listeners_for_clear
                        .borrow_mut()
                        .retain(|(known, _, _)| known != &channel);
                } else {
                    listeners_for_clear.borrow_mut().clear();
                }
                Ok(this)
            },
        ))),
    );
    let send_sync = realm.function("sendSync", 1, false, |realm, _, args| {
        let channel = ipc_channel(realm, args)?;
        let main_pointer = realm.electron_ipc_main;
        if main_pointer == 0 || main_pointer == realm as *mut Realm as usize {
            return Err(realm.throw(crate::ErrorKind::Error, "preload IPC host is unavailable"));
        }
        let result = {
            // The generated host installs this pointer only while the main
            // realm is alive; preload initialization and callbacks are run on
            // the same GUI thread after main-realm execution has yielded.
            // SAFETY: the host-owned main Realm outlives this preload Realm,
            // and all Realm access stays on that one thread.
            let main = unsafe { &mut *(main_pointer as *mut Realm) };
            let sender = realm.electron_window as u32;
            crate::electron_ipc::send_sync(
                main,
                realm,
                sender,
                &channel,
                args.get(1..).unwrap_or_default(),
            )
        };
        result.map_err(|message| realm.throw(crate::ErrorKind::Error, &message))
    });
    renderer.define(
        Key::name("sendSync"),
        Property::data(Value::Object(send_sync)),
    );

    let send = realm.function("send", 1, false, |realm, _, args| {
        let channel = ipc_channel(realm, args)?;
        let main_pointer = realm.electron_ipc_main;
        if main_pointer == 0 || main_pointer == realm as *mut Realm as usize {
            return Err(realm.throw(crate::ErrorKind::Error, "preload IPC host is unavailable"));
        }
        let result = {
            // SAFETY: the generated host owns this live main Realm for the
            // preload's full lifetime, and dispatch is single-threaded.
            let main = unsafe { &mut *(main_pointer as *mut Realm) };
            let sender = realm.electron_window as u32;
            crate::electron_ipc::send_sync(
                main,
                realm,
                sender,
                &channel,
                args.get(1..).unwrap_or_default(),
            )
        };
        result
            .map(|_| Value::Undefined)
            .map_err(|message| realm.throw(crate::ErrorKind::Error, &message))
    });
    renderer.define(Key::name("send"), Property::data(Value::Object(send)));

    // One message with the message ports it moves to the main realm.
    let post_message = realm.function("postMessage", 2, false, |realm, _, args| {
        let channel = ipc_channel(realm, args)?;
        let main_pointer = realm.electron_ipc_main;
        if main_pointer == 0 || main_pointer == realm as *mut Realm as usize {
            return Err(realm.throw(crate::ErrorKind::Error, "preload IPC host is unavailable"));
        }
        let message = args.get(1).cloned().unwrap_or_default();
        let transfer = args.get(2).cloned().unwrap_or_default();
        let moved = crate::realm_ports::transfer_list(realm, &transfer, None)?;
        crate::realm_ports::move_out(realm, &moved);
        let result = {
            // SAFETY: as for `send`.
            let main = unsafe { &mut *(main_pointer as *mut Realm) };
            let sender = realm.electron_window as u32;
            crate::electron_ipc::post_message(main, realm, sender, &channel, &message, &moved)
        };
        result
            .map(|()| Value::Undefined)
            .map_err(|message| realm.throw(crate::ErrorKind::Error, &message))
    });
    renderer.define(
        Key::name("postMessage"),
        Property::data(Value::Object(post_message)),
    );

    let invoke = realm.function("invoke", 1, false, |realm, _, args| {
        let channel = ipc_channel(realm, args)?;
        let main_pointer = realm.electron_ipc_main;
        let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
        if main_pointer == 0 || main_pointer == realm as *mut Realm as usize {
            let error = realm.throw(crate::ErrorKind::Error, "preload IPC host is unavailable");
            ops::call(realm, &reject, Value::Undefined, &[error.0])?;
            return Ok(promise);
        }
        let result = {
            // SAFETY: the generated host owns this live main Realm for the
            // preload's full lifetime, and dispatch is single-threaded.
            let main = unsafe { &mut *(main_pointer as *mut Realm) };
            let sender = realm.electron_window as u32;
            crate::electron_ipc::invoke(
                main,
                realm,
                sender,
                &channel,
                args.get(1..).unwrap_or_default(),
                (resolve.clone(), reject.clone()),
            )
        };
        if let Err(message) = result {
            let error = realm.throw(crate::ErrorKind::Error, &message);
            ops::call(realm, &reject, Value::Undefined, &[error.0])?;
        }
        Ok(promise)
    });
    renderer.define(Key::name("invoke"), Property::data(Value::Object(invoke)));
    renderer
}

fn ipc_channel(realm: &mut Realm, args: &[Value]) -> Result<String, Thrown> {
    let value = args.first().cloned().unwrap_or(Value::Undefined);
    let value = ops::to_string(realm, &value)?.to_rust_lossy();
    Ok(value)
}

fn web_frame(realm: &Realm) -> Object {
    let frame = realm.object();
    for (name, field) in [("setZoomFactor", 0), ("setZoomLevel", 1)] {
        let setter = realm.function(name, 1, false, move |realm, _, args| {
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            let value = ops::to_number(realm, &value)?;
            if !value.is_finite() || (field == 0 && value <= 0.0) {
                return Err(realm.throw(crate::ErrorKind::Range, "invalid zoom value"));
            }
            let factor = if field == 0 {
                value
            } else {
                crate::electron_window::zoom_factor(value)
            };
            // A page zooms its window, whose `webContents` reports it too.
            if realm.electron_window != 0 {
                let window = crate::app_host::Window::from_raw(realm.electron_window);
                crate::app_host::set_zoom(window, factor);
            }
            realm.electron_zoom_factor = factor;
            realm.electron_zoom_level = crate::electron_window::zoom_level(factor);
            Ok(Value::Undefined)
        });
        frame.define(Key::name(name), Property::data(Value::Object(setter)));
    }
    for (name, field) in [("getZoomFactor", 0), ("getZoomLevel", 1)] {
        let getter = realm.function(name, 0, false, move |realm, _, _| {
            let factor = if realm.electron_window != 0 {
                crate::app_host::zoom(crate::app_host::Window::from_raw(realm.electron_window))
            } else {
                realm.electron_zoom_factor
            };
            Ok(Value::Number(if field == 0 {
                factor
            } else {
                crate::electron_window::zoom_level(factor)
            }))
        });
        frame.define(Key::name(name), Property::data(Value::Object(getter)));
    }
    // Pages are never pinch-zoomed, as Electron's default limits of 1 and 1
    // keep them; the limits are numbers, as Electron's binding converts them.
    let limits = realm.function("setVisualZoomLevelLimits", 2, false, |realm, _, args| {
        for index in 0..2 {
            let value = args.get(index).cloned().unwrap_or(Value::Undefined);
            if !matches!(value, Value::Number(_)) {
                return Err(conversion_failure(realm, index, &value));
            }
        }
        Ok(Value::Undefined)
    });
    frame.define(
        Key::name("setVisualZoomLevelLimits"),
        Property::data(Value::Object(limits)),
    );
    frame
}

/// The TypeError Electron's bindings throw for an argument of the wrong
/// type, showing the value as gin's `V8TypeAsString` does: only strings,
/// `undefined` and `null` show.
pub(crate) fn conversion_failure(realm: &Realm, index: usize, value: &Value) -> crate::Thrown {
    let shown = match value {
        Value::Undefined => "undefined".into(),
        Value::Null => "null".into(),
        Value::String(text) => text.to_rust_lossy(),
        _ => alloc::string::String::new(),
    };
    realm.throw(
        crate::ErrorKind::Type,
        &alloc::format!(
            "Error processing argument at index {index}, conversion failure from {shown}"
        ),
    )
}

fn web_utils(realm: &Realm) -> Object {
    let utils = realm.object();
    let get_path = realm.function("getPathForFile", 1, false, |realm, _, args| {
        let file = args.first().cloned().unwrap_or(Value::Undefined);
        if matches!(file, Value::Undefined | Value::Null) {
            return Err(realm.throw(crate::ErrorKind::Type, "file must be a File object"));
        }
        let path = ops::get_named(realm, &file, "path")?;
        match path {
            Value::String(_) => Ok(path),
            Value::Undefined | Value::Null => Ok(Value::str("")),
            _ => Err(realm.throw(crate::ErrorKind::Type, "file path must be a string")),
        }
    });
    utils.define(
        Key::name("getPathForFile"),
        Property::data(Value::Object(get_path)),
    );
    utils
}

fn net(realm: &Realm) -> Object {
    let net = realm.object();
    let fetch = realm.function("fetch", 1, false, |realm, _, args| {
        crate::http_binding::fetch(realm, args)
    });
    net.define(Key::name("fetch"), Property::data(Value::Object(fetch)));
    // request(options[, callback]): Electron's ClientRequest, from the
    // Node.js library, with the application's user agent.
    let request = realm.function("request", 1, false, |realm, _, args| {
        let module = crate::modules::load(realm, "node:internal/electron/net")?;
        let create = match &module {
            Value::Object(_) => ops::get_named(realm, &module, "request")?,
            _ => Value::Undefined,
        };
        if !crate::array_builtin::is_callable(&create) {
            return Err(realm.throw(
                crate::ErrorKind::Error,
                "net.request needs the http and https modules, which this application does not link",
            ));
        }
        let agent = Value::str(&user_agent(realm));
        let options = args.first().cloned().unwrap_or(Value::Undefined);
        let callback = args.get(1).cloned().unwrap_or(Value::Undefined);
        ops::call(realm, &create, Value::Undefined, &[options, callback, agent])
    });
    net.define(Key::name("request"), Property::data(Value::Object(request)));
    // isOnline() and the `online` property: whether a network is connected.
    let online = realm.function("isOnline", 0, false, |_, _, _| {
        Ok(Value::Boolean(crate::tcp_binding::is_online()))
    });
    net.define(
        Key::name("isOnline"),
        Property::data(Value::Object(online.clone())),
    );
    net.define(
        Key::name("online"),
        Property {
            slot: crate::Slot::Accessor {
                get: Some(online),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    net
}

/// The `session` module and its default session.
fn session(realm: &Realm, protocol: &Object) -> (Object, Object) {
    let session = realm.object();
    let default_session = realm.object();
    default_session.define(
        Key::name("protocol"),
        Property::data(Value::Object(protocol.clone())),
    );
    let set_proxy = realm.function("setProxy", 1, false, |realm, _, args| {
        let options = args.first().cloned().unwrap_or(Value::Undefined);
        let mode = match ops::get_named(realm, &options, "mode")? {
            Value::Undefined | Value::Null => "system".into(),
            value => ops::to_string(realm, &value)?.to_rust_lossy(),
        };
        match mode.as_str() {
            "direct" => crate::http_binding::set_direct_proxy(true),
            "system" | "auto_detect" => crate::http_binding::set_direct_proxy(false),
            _ => {
                return Err(realm.throw(
                    crate::ErrorKind::Type,
                    "native sessions support only direct, system, and auto_detect proxy modes",
                ));
            }
        }
        let (promise, resolve, _) = crate::promise_builtin::capability(realm);
        ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
        Ok(promise)
    });
    default_session.define(
        Key::name("setProxy"),
        Property::data(Value::Object(set_proxy)),
    );
    // resolveProxy(url): the proxy list Chromium would use for the URL.
    let resolve_proxy = realm.function("resolveProxy", 1, false, |realm, _, args| {
        let url = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?.to_rust_lossy();
        let direct = crate::http_binding::direct_proxy();
        let (promise, resolve, _) = crate::promise_builtin::capability(realm);
        realm
            .submit_io(
                move || crate::system_proxy::resolve(&url, direct),
                move |realm, list| {
                    ops::call(realm, &resolve, Value::Undefined, &[Value::str(&list)])?;
                    Ok(())
                },
            )
            .map_err(|_| realm.throw(crate::ErrorKind::Error, "could not resolve the proxy"))?;
        Ok(promise)
    });
    default_session.define(
        Key::name("resolveProxy"),
        Property::data(Value::Object(resolve_proxy)),
    );
    preload_scripts(realm, &default_session);
    session_handlers(realm, &default_session);
    // Every partition shares the one native network stack, so each
    // partition's session is the default one.
    let shared = default_session.clone();
    let from_partition = realm.function("fromPartition", 1, false, move |realm, _, args| {
        if !matches!(args.first(), Some(Value::String(_))) {
            return Err(realm.throw(
                crate::ErrorKind::Type,
                "fromPartition requires a partition string",
            ));
        }
        Ok(Value::Object(shared.clone()))
    });
    session.define(
        Key::name("fromPartition"),
        Property::data(Value::Object(from_partition)),
    );
    // So is the session of a storage path.
    let shared = default_session.clone();
    let from_path = realm.function("fromPath", 1, false, move |realm, _, args| {
        if !matches!(args.first(), Some(Value::String(_))) {
            return Err(realm.throw(crate::ErrorKind::Type, "fromPath requires an absolute path"));
        }
        Ok(Value::Object(shared.clone()))
    });
    session.define(
        Key::name("fromPath"),
        Property::data(Value::Object(from_path)),
    );
    session.define(
        Key::name("defaultSession"),
        Property::data(Value::Object(default_session.clone())),
    );
    (session, default_session)
}

/// A session's handlers, caches and request hooks. Native pages ask no
/// permissions, download nothing through the session and load their files
/// without Chromium's network service, so handlers are accepted without
/// being called, caches have nothing to clear, and `webRequest` listeners
/// are kept but see no requests.
pub(crate) fn session_handlers(realm: &Realm, session: &Object) {
    for name in [
        "setPermissionRequestHandler",
        "setPermissionCheckHandler",
        "setDisplayMediaRequestHandler",
        "setDevicePermissionHandler",
        "setBluetoothPairingHandler",
        "setUSBProtectedClassesHandler",
        "setCertificateVerifyProc",
        "setSpellCheckerDictionaryDownloadURL",
        "setCodeCachePath",
        "setSSLConfig",
        "setUserAgent",
        "setDownloadPath",
        "allowNTLMCredentialsForDomains",
        "flushStorageData",
        "setPreconnect",
    ] {
        crate::electron_window::define(realm, session, name, 1, |_, _, _| Ok(Value::Undefined));
    }
    for name in [
        "clearData",
        "clearCache",
        "clearStorageData",
        "clearHostResolverCache",
        "clearAuthCache",
        "clearCodeCaches",
        "closeAllConnections",
    ] {
        crate::electron_window::define(realm, session, name, 1, |realm, _, _| {
            let (promise, resolve, _) = crate::promise_builtin::capability(realm);
            ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
            Ok(promise)
        });
    }
    crate::electron_window::define(realm, session, "getUserAgent", 0, |realm, _, _| {
        Ok(Value::str(&user_agent(realm)))
    });
    crate::electron_window::define(realm, session, "isPersistent", 0, |_, _, _| {
        Ok(Value::Boolean(true))
    });
    crate::electron_window::define(realm, session, "getStoragePath", 0, |realm, _, _| {
        app_path(realm, "userData")
    });
    // `fetch` is the main process's, which the native network stack serves.
    crate::electron_window::define(realm, session, "fetch", 2, |realm, _, args| {
        let global = Value::Object(realm.global.clone());
        let fetch = ops::get_named(realm, &global, "fetch")?;
        if !crate::array_builtin::is_callable(&fetch) {
            return Err(realm.throw(
                crate::ErrorKind::Error,
                "session.fetch needs fetch, which this application does not link",
            ));
        }
        ops::call(realm, &fetch, Value::Undefined, args)
    });
    let web_request = realm.object();
    for name in [
        "onBeforeRequest",
        "onBeforeSendHeaders",
        "onSendHeaders",
        "onHeadersReceived",
        "onResponseStarted",
        "onBeforeRedirect",
        "onCompleted",
        "onErrorOccurred",
    ] {
        crate::electron_window::define(realm, &web_request, name, 2, |_, _, _| {
            Ok(Value::Undefined)
        });
    }
    session.define(
        Key::name("webRequest"),
        Property::data(Value::Object(web_request)),
    );
}

/// A preload script registered with a session: its id, type and file.
type PreloadScript = (String, String, String);

/// A session's preload registry: `registerPreloadScript` and the older
/// `setPreloads`. Compiled windows run the preloads compiled with them, so
/// scripts a program registers or writes while running are recorded for
/// the program but not run.
fn preload_scripts(realm: &Realm, session: &Object) {
    let scripts = Rc::new(RefCell::new(Vec::<PreloadScript>::new()));
    let registered = scripts.clone();
    let register = realm.function("registerPreloadScript", 1, false, move |realm, _, args| {
        let script = args.first().cloned().unwrap_or(Value::Undefined);
        if script.as_object().is_none() {
            return Err(realm.throw(
                crate::ErrorKind::Type,
                "registerPreloadScript requires a script object",
            ));
        }
        let text = |realm: &mut Realm, name: &str| -> Result<Option<String>, crate::Thrown> {
            match ops::get(realm, &script, &Key::name(name))? {
                Value::Undefined => Ok(None),
                value => Ok(Some(ops::to_string(realm, &value)?.to_rust_lossy())),
            }
        };
        let file_path = text(realm, "filePath")?.unwrap_or_default();
        let kind = text(realm, "type")?.unwrap_or_else(|| "frame".to_owned());
        let id = text(realm, "id")?;
        let mut scripts = registered.borrow_mut();
        let id = id.unwrap_or_else(|| format!("nativeboat-preload-{}", scripts.len() + 1));
        if scripts.iter().any(|(known, _, _)| *known == id) {
            return Err(realm.throw(
                crate::ErrorKind::Error,
                &format!("Cannot register preload script with existing ID '{id}'"),
            ));
        }
        scripts.push((id.clone(), kind, file_path));
        Ok(Value::str(&id))
    });
    let unregistered = scripts.clone();
    let unregister = realm.function(
        "unregisterPreloadScript",
        1,
        false,
        move |realm, _, args| {
            let id =
                ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?.to_rust_lossy();
            unregistered
                .borrow_mut()
                .retain(|(known, _, _)| *known != id);
            Ok(Value::Undefined)
        },
    );
    let listed = scripts.clone();
    let get_scripts = realm.function("getPreloadScripts", 0, false, move |realm, _, _| {
        let entries = listed
            .borrow()
            .iter()
            .map(|(id, kind, file_path)| {
                let entry = realm.object();
                for (name, value) in [("filePath", file_path), ("id", id), ("type", kind)] {
                    entry.define(Key::name(name), Property::data(Value::str(value)));
                }
                Value::Object(entry)
            })
            .collect();
        Ok(Value::Object(realm.array(entries)))
    });
    // `setPreloads` replaces the frame scripts it set before.
    let replaced = scripts.clone();
    let set_preloads = realm.function("setPreloads", 1, false, move |realm, _, args| {
        let list = args.first().cloned().unwrap_or(Value::Undefined);
        let mut files = Vec::new();
        let mut iterator = crate::iter::iterate(realm, &list)?;
        while let Some(value) = iterator.step(realm)? {
            files.push(ops::to_string(realm, &value)?.to_rust_lossy());
        }
        let mut scripts = replaced.borrow_mut();
        scripts.retain(|(id, _, _)| !id.starts_with("nativeboat-set-preload-"));
        for (index, file) in files.into_iter().enumerate() {
            scripts.push((
                format!("nativeboat-set-preload-{index}"),
                "frame".to_owned(),
                file,
            ));
        }
        Ok(Value::Undefined)
    });
    let preloads = scripts;
    let get_preloads = realm.function("getPreloads", 0, false, move |realm, _, _| {
        let files = preloads
            .borrow()
            .iter()
            .filter(|(id, _, _)| id.starts_with("nativeboat-set-preload-"))
            .map(|(_, _, file)| Value::str(file))
            .collect();
        Ok(Value::Object(realm.array(files)))
    });
    for (name, function) in [
        ("registerPreloadScript", register),
        ("unregisterPreloadScript", unregister),
        ("getPreloadScripts", get_scripts),
        ("setPreloads", set_preloads),
        ("getPreloads", get_preloads),
    ] {
        session.define(Key::name(name), Property::hidden(Value::Object(function)));
    }
}

#[derive(Clone)]
struct ThemeListener {
    event: String,
    callback: Object,
    once: bool,
}

fn native_theme(realm: &Realm) -> Object {
    let theme = realm.object();
    let source = Rc::new(RefCell::new("system".to_owned()));
    let listeners = Rc::new(RefCell::new(Vec::<ThemeListener>::new()));

    let theme_source = source.clone();
    let get_source = realm.function("get themeSource", 0, false, move |_, _, _| {
        Ok(Value::str(&theme_source.borrow()))
    });
    let theme_source = source.clone();
    let theme_for_setter = theme.clone();
    let listeners_for_setter = listeners.clone();
    let set_source = realm.function("set themeSource", 1, false, move |realm, _, args| {
        let value = args.first().cloned().unwrap_or(Value::Undefined);
        let value = ops::to_string(realm, &value)?.to_rust_lossy();
        if !matches!(value.as_str(), "system" | "light" | "dark") {
            return Err(realm.throw(
                crate::ErrorKind::Type,
                "nativeTheme.themeSource must be 'system', 'light', or 'dark'",
            ));
        }
        let changed = {
            let mut current = theme_source.borrow_mut();
            if *current == value {
                false
            } else {
                *current = value;
                true
            }
        };
        if changed {
            emit_theme_event(realm, &theme_for_setter, &listeners_for_setter, "updated")?;
        }
        Ok(Value::Undefined)
    });
    ops::define_accessor(
        &theme,
        Key::name("themeSource"),
        Some(get_source),
        Some(set_source),
    );

    let dark_source = source.clone();
    let dark = realm.function("get shouldUseDarkColors", 0, false, move |_, _, _| {
        Ok(Value::Boolean(match dark_source.borrow().as_str() {
            "dark" => true,
            "light" => false,
            _ => app_host::dark_mode(),
        }))
    });
    ops::define_accessor(&theme, Key::name("shouldUseDarkColors"), Some(dark), None);
    for name in ["shouldUseHighContrastColors", "inForcedColorsMode"] {
        let getter = realm.function(name, 0, false, |_, _, _| Ok(Value::Boolean(false)));
        ops::define_accessor(&theme, Key::name(name), Some(getter), None);
    }

    for (name, once) in [("on", false), ("addListener", false), ("once", true)] {
        let listeners = listeners.clone();
        let method = realm.function(name, 2, false, move |realm, this, args| {
            let event = args.first().cloned().unwrap_or(Value::Undefined);
            let event = ops::to_string(realm, &event)?.to_rust_lossy();
            let callback = match args.get(1) {
                Some(Value::Object(callback))
                    if crate::array_builtin::is_callable(&Value::Object(callback.clone())) =>
                {
                    callback.clone()
                }
                _ => {
                    return Err(realm.throw(
                        crate::ErrorKind::Type,
                        "The listener argument must be a function",
                    ));
                }
            };
            listeners.borrow_mut().push(ThemeListener {
                event,
                callback,
                once,
            });
            Ok(this)
        });
        theme.define(Key::name(name), Property::data(Value::Object(method)));
    }

    let listeners_for_count = listeners.clone();
    let count = realm.function("listenerCount", 1, false, move |realm, _, args| {
        let event = args.first().cloned().unwrap_or(Value::Undefined);
        let event = ops::to_string(realm, &event)?.to_rust_lossy();
        Ok(Value::Number(
            listeners_for_count
                .borrow()
                .iter()
                .filter(|listener| listener.event == event)
                .count() as f64,
        ))
    });
    theme.define(
        Key::name("listenerCount"),
        Property::data(Value::Object(count)),
    );

    let remove = {
        let listeners = listeners.clone();
        realm.function("removeListener", 2, false, move |realm, this, args| {
            let event = args.first().cloned().unwrap_or(Value::Undefined);
            let event = ops::to_string(realm, &event)?.to_rust_lossy();
            let callback = args.get(1).cloned().unwrap_or(Value::Undefined);
            let mut listeners = listeners.borrow_mut();
            if let Some(index) = listeners.iter().rposition(|listener| {
                listener.event == event
                    && crate::same_value(&callback, &Value::Object(listener.callback.clone()))
            }) {
                listeners.remove(index);
            }
            Ok(this)
        })
    };
    theme.define(
        Key::name("removeListener"),
        Property::data(Value::Object(remove.clone())),
    );
    theme.define(Key::name("off"), Property::data(Value::Object(remove)));
    let remove_all = {
        let listeners = listeners.clone();
        realm.function("removeAllListeners", 1, false, move |realm, this, args| {
            if let Some(event) = args.first() {
                let event = ops::to_string(realm, event)?.to_rust_lossy();
                listeners
                    .borrow_mut()
                    .retain(|listener| listener.event != event);
            } else {
                listeners.borrow_mut().clear();
            }
            Ok(this)
        })
    };
    theme.define(
        Key::name("removeAllListeners"),
        Property::data(Value::Object(remove_all)),
    );
    let emit = {
        let listeners = listeners.clone();
        let theme = theme.clone();
        realm.function("emit", 1, false, move |realm, _, args| {
            let event = args.first().cloned().unwrap_or(Value::Undefined);
            let event = ops::to_string(realm, &event)?.to_rust_lossy();
            emit_theme_event(realm, &theme, &listeners, &event).map(Value::Boolean)
        })
    };
    theme.define(Key::name("emit"), Property::data(Value::Object(emit)));
    theme
}

fn emit_theme_event(
    realm: &mut Realm,
    theme: &Object,
    listeners: &Rc<RefCell<Vec<ThemeListener>>>,
    event: &str,
) -> Result<bool, Thrown> {
    let callbacks = {
        let mut listeners = listeners.borrow_mut();
        let callbacks = listeners
            .iter()
            .filter(|listener| listener.event == event)
            .map(|listener| listener.callback.clone())
            .collect::<Vec<_>>();
        listeners.retain(|listener| listener.event != event || !listener.once);
        callbacks
    };
    for callback in &callbacks {
        ops::call(
            realm,
            &Value::Object(callback.clone()),
            Value::Object(theme.clone()),
            &[],
        )?;
    }
    Ok(!callbacks.is_empty())
}

fn app(realm: &Realm) -> Object {
    let app = realm.object();
    app.define(
        Key::name("isPackaged"),
        Property::data(Value::Boolean(true)),
    );
    app.define(
        Key::name("getAppPath"),
        Property::data(Value::Object(realm.function(
            "getAppPath",
            0,
            false,
            // Without the trailing separator, as Electron gives it.
            |realm, _, _| {
                let Value::String(path) = crate::process_builtin::app_path(realm)? else {
                    unreachable!("the application path is a string")
                };
                let units = path.units();
                let end = units.len()
                    - usize::from(units.last().is_some_and(|unit| *unit == u16::from(b'\\')));
                Ok(Value::String(crate::JsStr::from_units(&units[..end])))
            },
        ))),
    );
    app.define(
        Key::name("getVersion"),
        Property::data(Value::Object(realm.function(
            "getVersion",
            0,
            false,
            |realm, _, _| Ok(Value::str(realm.electron_app_version.unwrap_or("0.0.0"))),
        ))),
    );
    app.define(
        Key::name("getPath"),
        Property::data(Value::Object(realm.function(
            "getPath",
            1,
            false,
            get_app_path,
        ))),
    );
    app.define(
        Key::name("setPath"),
        Property::data(Value::Object(realm.function(
            "setPath",
            2,
            false,
            set_app_path,
        ))),
    );
    app.define(
        Key::name("getName"),
        Property::data(Value::Object(realm.function(
            "getName",
            0,
            false,
            |realm, _, _| {
                Ok(Value::str(
                    realm.electron_app_name.unwrap_or("Nativeboat App"),
                ))
            },
        ))),
    );
    for (name, length) in [
        ("disableHardwareAcceleration", 0),
        ("releaseSingleInstanceLock", 0),
        ("addRecentDocument", 1),
        ("clearRecentDocuments", 0),
        ("relaunch", 1),
        ("setJumpList", 1),
    ] {
        let function = realm.function(name, length, false, |_, _, _| Ok(Value::Undefined));
        app.define(Key::name(name), Property::data(Value::Object(function)));
    }
    crate::electron_single_instance::install(realm, &app);
    crate::electron_login_item::install(realm, &app);
    crate::electron_app::install(realm, &app);
    app.define(
        Key::name("getLocale"),
        Property::data(Value::Object(realm.function(
            "getLocale",
            0,
            false,
            |_, _, _| Ok(Value::str(&crate::platform::user_locale())),
        ))),
    );
    app.define(
        Key::name("getSystemLocale"),
        Property::data(Value::Object(realm.function(
            "getSystemLocale",
            0,
            false,
            |_, _, _| Ok(Value::str(&crate::platform::system_locale())),
        ))),
    );
    app.define(
        Key::name("getPreferredSystemLanguages"),
        Property::data(Value::Object(realm.function(
            "getPreferredSystemLanguages",
            0,
            false,
            |realm, _, _| {
                let languages = crate::platform::preferred_languages()
                    .iter()
                    .map(|language| Value::str(language))
                    .collect();
                Ok(Value::Object(realm.array(languages)))
            },
        ))),
    );
    let command_line = realm.object();
    for (name, length) in [("appendSwitch", 2), ("appendArgument", 1)] {
        let function = realm.function(name, length, false, |_, _, _| Ok(Value::Undefined));
        command_line.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let has_switch = realm.function("hasSwitch", 1, false, |_, _, _| Ok(Value::Boolean(false)));
    command_line.define(
        Key::name("hasSwitch"),
        Property::data(Value::Object(has_switch)),
    );
    let get_switch = realm.function("getSwitchValue", 1, false, |_, _, _| Ok(Value::str("")));
    command_line.define(
        Key::name("getSwitchValue"),
        Property::data(Value::Object(get_switch)),
    );
    app.define(
        Key::name("commandLine"),
        Property::data(Value::Object(command_line)),
    );
    let ready = realm.function("whenReady", 0, false, |realm, _, _| {
        let (promise, resolve, _) = crate::promise_builtin::capability(realm);
        ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
        Ok(promise)
    });
    app.define(Key::name("whenReady"), Property::data(Value::Object(ready)));
    app.define(
        Key::name("isReady"),
        Property::data(Value::Object(realm.function(
            "isReady",
            0,
            false,
            |_, _, _| Ok(Value::Boolean(true)),
        ))),
    );
    app.define(
        Key::name("quit"),
        Property::data(Value::Object(realm.function(
            "quit",
            0,
            false,
            |realm, _, _| {
                crate::electron_window::quit(realm)?;
                Ok(Value::Undefined)
            },
        ))),
    );
    app.define(
        Key::name("exit"),
        Property::data(Value::Object(realm.function(
            "exit",
            1,
            false,
            |_, _, args| {
                let code = args
                    .first()
                    .and_then(|value| match value {
                        Value::Number(value) => Some(*value as i32),
                        _ => None,
                    })
                    .unwrap_or(0);
                app_host::request_quit(code);
                Ok(Value::Undefined)
            },
        ))),
    );
    app
}

/// Without the Node.js library, `app.on('ready')` listeners run after the
/// main script and other listeners are ignored.
fn install_ready_listener(realm: &Realm, app: &Object) {
    crate::electron_window::inert_listeners(realm, app);
    let receiver = Value::Object(app.clone());
    app.define(
        Key::name("on"),
        Property::data(Value::Object(realm.function(
            "on",
            2,
            false,
            move |realm, this, args| {
                if matches!(args.first(), Some(Value::String(name)) if name.units() == "ready".encode_utf16().collect::<Vec<_>>())
                    && let Some(listener) = args.get(1).cloned()
                {
                    let receiver = receiver.clone();
                    realm.enqueue(Box::new(move |realm| {
                        ops::call(realm, &listener, receiver, &[]).map(|_| ())
                    }));
                }
                Ok(this)
            },
        ))),
    );
}

/// The file that keeps the `localStorage` of the application's pages, in
/// its user data directory, which this creates when it is missing; as
/// Electron keeps it in `Local Storage` there.
pub fn page_storage_file(realm: &mut Realm) -> Option<String> {
    let Ok(Value::String(directory)) = app_path(realm, "userData") else {
        return None;
    };
    let directory = directory.to_rust_lossy();
    let storage = join_windows_path(&directory, "Local Storage");
    for path in [&directory, &storage] {
        let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
        // An existing directory is fine.
        let _ = crate::fs_win::mkdir(&wide);
    }
    Some(join_windows_path(&storage, "nativeboat.json"))
}

/// The directory a `--user-data-dir` switch names, which Electron makes the
/// `userData` path: absolute, and created with the directories above it.
/// `arguments` is the command line, the program first. An empty value
/// leaves the default, as in Electron.
fn user_data_dir(realm: &Realm, arguments: &[crate::JsStr]) -> Result<Option<String>, Thrown> {
    let Some(value) = switch_value(arguments.get(1..).unwrap_or_default(), "user-data-dir")
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let (units, text) = crate::fs_binding::full_path(realm, &Value::str(&value))?;
    // Each directory above it, then itself; those that exist stay.
    for (end, unit) in units.iter().enumerate() {
        if matches!(unit, 0x5C | 0x2F | 0) && end > 3 {
            let mut prefix = units[..end].to_vec();
            prefix.push(0);
            let _ = crate::fs_win::mkdir(&prefix);
        }
    }
    Ok(Some(text.to_rust_lossy()))
}

/// The value of a command-line switch as Chromium reads it on Windows:
/// `--name=value`, `-name=value` or `/name=value`, its name in any case,
/// the last one counting, before a `--` that ends the switches. A switch
/// without a value has an empty one.
fn switch_value(arguments: &[crate::JsStr], name: &str) -> Option<String> {
    let mut found = None;
    for argument in arguments {
        let argument = argument.to_rust_lossy();
        if argument == "--" {
            break;
        }
        let Some(rest) = ["--", "-", "/"]
            .into_iter()
            .find_map(|prefix| argument.strip_prefix(prefix))
        else {
            continue;
        };
        let (key, value) = rest.split_once('=').unwrap_or((rest, ""));
        if key.eq_ignore_ascii_case(name) {
            found = Some(value.to_string());
        }
    }
    found
}

/// `app.getPath(name)`.
pub(crate) fn app_path(realm: &mut Realm, name: &str) -> Result<Value, Thrown> {
    get_app_path(realm, Value::Undefined, &[Value::str(name)])
}

fn get_app_path(realm: &mut Realm, _: Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = args
        .first()
        .map(|value| ops::to_string(realm, value))
        .transpose()?
        .unwrap_or_else(crate::JsStr::default)
        .to_rust_lossy();
    if let Some((_, path)) = realm
        .electron_app_paths
        .iter()
        .find(|(configured_name, _)| configured_name == &name)
    {
        return Ok(Value::str(path));
    }
    let environment = crate::environment_builtin::environment(realm);
    let env_path = |realm: &mut Realm, key: &str| -> Result<Option<String>, Thrown> {
        let value = ops::get(realm, &environment, &Key::name(key))?;
        Ok(match value {
            Value::String(value) => Some(value.to_rust_lossy()),
            _ => None,
        })
    };
    let home = env_path(realm, "USERPROFILE")?;
    let app_data = env_path(realm, "APPDATA")?.or_else(|| {
        home.as_ref()
            .map(|home| join_windows_path(home, "AppData\\Roaming"))
    });
    let switched = user_data_dir(realm, &crate::platform::command_arguments())?;
    let user_data = switched.or_else(|| {
        app_data.as_ref().map(|app_data| {
            join_windows_path(
                app_data,
                realm.electron_app_name.unwrap_or("Nativeboat App"),
            )
        })
    });
    let path = match name.as_str() {
        "userData" => user_data,
        "appData" => app_data,
        "home" => home,
        "temp" => env_path(realm, "TEMP")?,
        "desktop" => home.as_ref().map(|home| join_windows_path(home, "Desktop")),
        "documents" => home
            .as_ref()
            .map(|home| join_windows_path(home, "Documents")),
        "downloads" => home
            .as_ref()
            .map(|home| join_windows_path(home, "Downloads")),
        "logs" => user_data.map(|path| join_windows_path(&path, "logs")),
        "crashDumps" => user_data.map(|path| join_windows_path(&path, "Crashpad")),
        "sessionData" => user_data,
        "exe" | "module" => {
            let process = crate::process_builtin::process(realm)?;
            match ops::get_named(realm, &process, "execPath")? {
                Value::String(path) => Some(path.to_rust_lossy()),
                _ => None,
            }
        }
        _ => {
            return Err(realm.throw(
                crate::ErrorKind::Type,
                &format!("Path name '{name}' is not supported by the native app host"),
            ));
        }
    };
    Ok(path.map_or(Value::Undefined, |path| Value::str(&path)))
}

fn set_app_path(realm: &mut Realm, _: Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = args
        .first()
        .map(|value| ops::to_string(realm, value))
        .transpose()?
        .unwrap_or_else(crate::JsStr::default)
        .to_rust_lossy();
    let path = args
        .get(1)
        .map(|value| ops::to_string(realm, value))
        .transpose()?
        .unwrap_or_else(crate::JsStr::default)
        .to_rust_lossy();
    if path.is_empty() {
        return Err(realm.throw(
            crate::ErrorKind::Type,
            "The path must be a non-empty string",
        ));
    }
    if let Some((_, configured_path)) = realm
        .electron_app_paths
        .iter_mut()
        .find(|(configured_name, _)| configured_name == &name)
    {
        *configured_path = path;
    } else {
        realm.electron_app_paths.push((name, path));
    }
    Ok(Value::Undefined)
}

fn join_windows_path(base: &str, child: &str) -> String {
    let mut path = base.trim_end_matches(['\\', '/']).to_string();
    path.push('\\');
    path.push_str(child.trim_start_matches(['\\', '/']));
    path
}

fn browser_window_constructor(realm: &mut Realm, emitter: Option<Value>) -> Object {
    let prototype = realm.object();
    match &emitter {
        Some(emitter) => {
            if let Ok(Value::Object(base)) = ops::get_named(realm, emitter, "prototype") {
                prototype.set_prototype(Some(base));
            }
        }
        None => crate::electron_window::inert_listeners(realm, &prototype),
    }
    let instance_emitter = emitter.clone();
    let instance_prototype = prototype.clone();
    let windows = Rc::new(RefCell::new(Vec::<(app_host::Window, Object)>::new()));
    let constructed_windows = windows.clone();
    let constructor = realm.builtin_constructor(
        "BrowserWindow",
        1,
        &prototype,
        |realm, _, _| {
            Err(realm.throw(
                crate::ErrorKind::Type,
                "BrowserWindow must be constructed with new",
            ))
        },
        move |realm, target, args| {
            // The construct callback receives `new.target`; the window is a new
            // object with its prototype.
            let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
                Value::Object(prototype) => prototype,
                _ => instance_prototype.clone(),
            };
            let instance = Object::new(Some(prototype), crate::Kind::Ordinary);
            let options = args.first().cloned().unwrap_or(Value::Undefined);
            let title = if let Value::Object(_) = options {
                match ops::get_named(realm, &options, "title")? {
                    Value::String(title) => title.units().to_vec(),
                    _ => app_title(realm),
                }
            } else if matches!(options, Value::Undefined | Value::Null) {
                app_title(realm)
            } else {
                return Err(realm.throw(
                    crate::ErrorKind::Type,
                    "BrowserWindow options must be an object",
                ));
            };
            let window_options = window_options(realm, &options, &title)?;
            let host_window = app_host::create(&window_options).map_err(|code| {
                realm.throw(
                    crate::ErrorKind::Error,
                    &format!("cannot create native window (Win32 error {code})"),
                )
            })?;
            // The window's own icon: a NativeImage or an image file.
            if matches!(options, Value::Object(_)) {
                let icon = ops::get_named(realm, &options, "icon")?;
                if !matches!(icon, Value::Undefined | Value::Null) {
                    let icon = crate::electron_notify::image_icon(realm, &icon);
                    app_host::set_window_icon(host_window, icon);
                }
            }
            let web_preferences = if matches!(options, Value::Object(_)) {
                ops::get_named(realm, &options, "webPreferences")?
            } else {
                Value::Undefined
            };
            if !matches!(web_preferences, Value::Undefined | Value::Null) {
                // Arguments the window's renderer gets, after its own.
                let additional = ops::get_named(realm, &web_preferences, "additionalArguments")?;
                if let Value::Object(_) = additional {
                    let length = ops::get_named(realm, &additional, "length")?;
                    let length = ops::to_number(realm, &length)? as u32;
                    let mut arguments = Vec::new();
                    for index in 0..length {
                        let argument = ops::get(realm, &additional, &Key::Index(index))?;
                        arguments.push(ops::to_string(realm, &argument)?.to_rust_lossy());
                    }
                    realm.register_electron_window_arguments(host_window.raw(), arguments);
                }
                let preload = ops::get_named(realm, &web_preferences, "preload")?;
                match preload {
                    Value::String(path) => {
                        realm.register_electron_window_preload(
                            host_window.raw(),
                            path.to_rust_lossy(),
                        );
                    }
                    Value::Undefined | Value::Null => {}
                    _ => {
                        let _ = app_host::destroy_window(host_window);
                        return Err(realm.throw(
                            crate::ErrorKind::Type,
                            "webPreferences.preload must be a string",
                        ));
                    }
                }
            }
            if let Some(emitter) = &instance_emitter {
                ops::call(realm, emitter, Value::Object(instance.clone()), &[])?;
            }
            instance.define(
                Key::name("id"),
                Property::data(Value::Number(host_window.raw() as f64)),
            );
            crate::electron_menu::attach_application_menu(realm, host_window)?;
            if window_options.content_size {
                // The menu bar, now attached, is part of the frame.
                app_host::set_content_size(
                    host_window,
                    window_options.size.0,
                    window_options.size.1,
                );
            }
            install_window_methods(
                realm,
                &instance,
                host_window,
                constructed_windows.clone(),
                instance_emitter.as_ref(),
            )?;
            constructed_windows
                .borrow_mut()
                .push((host_window, instance.clone()));
            Ok(instance)
        },
    );
    let live_windows = windows.clone();
    let get_all_windows = realm.function("getAllWindows", 0, false, move |realm, _, _| {
        let windows = live_windows
            .borrow()
            .iter()
            .filter(|(window, _)| app_host::is_window(*window))
            .map(|(_, instance)| Value::Object(instance.clone()))
            .collect();
        Ok(Value::Object(realm.array(windows)))
    });
    constructor.define(
        Key::name("getAllWindows"),
        Property::data(Value::Object(get_all_windows)),
    );
    let live_windows = windows;
    let from_id = realm.function("fromId", 1, false, move |realm, _, args| {
        let id = ops::to_number(realm, args.first().unwrap_or(&Value::Undefined))?;
        let window = live_windows
            .borrow()
            .iter()
            .find(|(window, _)| window.raw() as f64 == id && app_host::is_window(*window))
            .map(|(_, instance)| instance.clone());
        Ok(window.map_or(Value::Null, Value::Object))
    });
    constructor.define(Key::name("fromId"), Property::data(Value::Object(from_id)));
    let from_contents = realm.function("fromWebContents", 1, false, |realm, _, args| {
        let found = match args.first() {
            Some(Value::Object(contents)) => realm
                .electron_windows
                .iter()
                .find(|(_, _, known)| known == contents)
                .map(|(_, instance, _)| instance.clone()),
            _ => None,
        };
        Ok(found.map_or(Value::Null, Value::Object))
    });
    constructor.define(
        Key::name("fromWebContents"),
        Property::data(Value::Object(from_contents)),
    );
    let focused = realm.function("getFocusedWindow", 0, false, |realm, _, _| {
        let found = realm
            .electron_windows
            .iter()
            .find(|(window, _, _)| app_host::window_flags(app_host::Window::from_raw(*window)).2)
            .map(|(_, instance, _)| instance.clone());
        Ok(found.map_or(Value::Null, Value::Object))
    });
    constructor.define(
        Key::name("getFocusedWindow"),
        Property::data(Value::Object(focused)),
    );
    constructor
}

/// Electron's BrowserWindow options that shape the native window.
fn window_options<'a>(
    realm: &mut Realm,
    options: &Value,
    title: &'a [u16],
) -> Result<app_host::WindowOptions<'a>, Thrown> {
    let mut window = app_host::WindowOptions::new(title);
    if !matches!(options, Value::Object(_)) {
        return Ok(window);
    }
    let mut number = |name: &str| -> Result<Option<f64>, Thrown> {
        Ok(match ops::get(realm, options, &Key::name(name))? {
            Value::Number(value) if value.is_finite() => Some(value),
            _ => None,
        })
    };
    if let (Some(x), Some(y)) = (number("x")?, number("y")?) {
        window.position = Some((x, y));
    }
    let positive =
        |value: Option<f64>, default: f64| value.filter(|value| *value > 0.0).unwrap_or(default);
    window.size = (
        positive(number("width")?, 800.0),
        positive(number("height")?, 600.0),
    );
    window.minimum = (
        positive(number("minWidth")?, 0.0),
        positive(number("minHeight")?, 0.0),
    );
    window.maximum = (
        positive(number("maxWidth")?, 0.0),
        positive(number("maxHeight")?, 0.0),
    );
    let mut flag = |name: &str, default: bool| -> Result<bool, Thrown> {
        Ok(match ops::get(realm, options, &Key::name(name))? {
            Value::Undefined => default,
            value => ops::to_boolean(&value),
        })
    };
    window.content_size = flag("useContentSize", false)?;
    window.center = flag("center", false)?;
    window.frame = flag("frame", true)?;
    window.auto_hide_menu_bar = flag("autoHideMenuBar", false)?;
    window.resizable = flag("resizable", true)?;
    window.show = flag("show", true)?;
    window.transparent = flag("transparent", false)?;
    window.always_on_top = flag("alwaysOnTop", false)?;
    window.skip_taskbar = flag("skipTaskbar", false)?;
    if let Value::String(color) = ops::get_named(realm, options, "backgroundColor")?
        && let Some(color) = hex_color(&color.to_rust_lossy())
    {
        window.background = color;
    }
    Ok(window)
}

/// The title of windows that name none: the application's name, as Electron
/// does.
pub(crate) fn app_title(realm: &Realm) -> Vec<u16> {
    realm
        .electron_app_name
        .unwrap_or("Nativeboat App")
        .encode_utf16()
        .collect()
}

/// A `#rgb`, `#rrggbb` or `#aarrggbb` color, as Electron accepts them.
fn hex_color(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        3 => {
            let channel = |shift: u32| ((value >> shift) & 0xf) as u8 * 17;
            Some([channel(8), channel(4), channel(0)])
        }
        6 | 8 => Some([(value >> 16) as u8, (value >> 8) as u8, value as u8]),
        _ => None,
    }
}

fn install_window_methods(
    realm: &mut Realm,
    instance: &Object,
    window: app_host::Window,
    windows: Rc<RefCell<Vec<(app_host::Window, Object)>>>,
    emitter: Option<&Value>,
) -> Result<(), Thrown> {
    crate::electron_window::install_window_state_methods(realm, instance, window);
    instance.define(
        Key::Symbol(realm.electron_window_key.clone()),
        Property::hidden(Value::Number(window.raw() as f64)),
    );
    let set_menu = realm.function("setMenu", 1, false, move |realm, _, args| {
        let menu = args.first().cloned().unwrap_or(Value::Undefined);
        crate::electron_menu::set_window_menu(realm, window, menu)?;
        Ok(Value::Undefined)
    });
    instance.define(
        Key::name("setMenu"),
        Property::data(Value::Object(set_menu)),
    );
    let remove_menu = realm.function("removeMenu", 0, false, move |realm, _, _| {
        crate::electron_menu::set_window_menu(realm, window, Value::Null)?;
        Ok(Value::Undefined)
    });
    instance.define(
        Key::name("removeMenu"),
        Property::data(Value::Object(remove_menu)),
    );
    for (name, action) in [
        ("show", WindowAction::Show),
        ("hide", WindowAction::Hide),
        ("close", WindowAction::Close),
        ("destroy", WindowAction::Destroy),
    ] {
        let windows = windows.clone();
        let method = realm.function(name, 0, false, move |realm, _, _| {
            match action {
                WindowAction::Show => app_host::show_window(window),
                WindowAction::Hide => app_host::hide_window(window),
                // Closing asks first: the window's `close` listeners may
                // refuse, as for its close button.
                WindowAction::Close => app_host::request_close(window),
                WindowAction::Destroy => {
                    let result = app_host::destroy_window(window);
                    if result.is_ok() {
                        windows
                            .borrow_mut()
                            .retain(|(candidate, _)| *candidate != window);
                        crate::electron_menu::forget_window_menu(realm, window);
                    }
                    result
                }
            }
            .map_err(|code| {
                Thrown(Value::str(&format!(
                    "Win32 window operation failed ({code})"
                )))
            })?;
            Ok(Value::Undefined)
        });
        instance.define(Key::name(name), Property::data(Value::Object(method)));
    }
    for name in ["loadFile", "loadURL"] {
        let method = realm.function(name, 1, false, move |realm, _, args| {
            let Some(Value::String(target)) = args.first() else {
                return Err(realm.throw(
                    crate::ErrorKind::Type,
                    "BrowserWindow page target must be a string",
                ));
            };
            let mut target = target.to_rust_lossy();
            // As in Electron, a relative file is in the application folder.
            if name == "loadFile" && !absolute_path(&target) {
                let Value::String(folder) = crate::process_builtin::app_relative(realm, "")? else {
                    unreachable!("the application path is a string")
                };
                target = format!("{}\\{}", folder.to_rust_lossy(), target.replace('/', "\\"));
            }
            realm.register_electron_window_load(window.raw(), target);
            let (promise, resolve, _) = crate::promise_builtin::capability(realm);
            // As in Electron, the promise settles once the page has loaded.
            if realm.electron_hosts_pages {
                realm.electron_load_waiters.push((window.raw(), resolve));
            } else {
                ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
            }
            Ok(promise)
        });
        instance.define(Key::name(name), Property::data(Value::Object(method)));
    }
    let visible = realm.function("isVisible", 0, false, move |_, _, _| {
        Ok(Value::Boolean(app_host::is_visible(window)))
    });
    instance.define(
        Key::name("isVisible"),
        Property::data(Value::Object(visible)),
    );
    let destroyed = realm.function("isDestroyed", 0, false, move |_, _, _| {
        Ok(Value::Boolean(!app_host::is_window(window)))
    });
    instance.define(
        Key::name("isDestroyed"),
        Property::data(Value::Object(destroyed)),
    );
    let contents = crate::electron_window::web_contents(realm, emitter, window)?;
    // The contents load and close their window's page.
    for (name, from) in [
        ("loadURL", "loadURL"),
        ("loadFile", "loadFile"),
        ("close", "close"),
    ] {
        if let Some(crate::object::Property {
            slot: crate::object::Slot::Data { value, .. },
            ..
        }) = instance.own_property(&Key::name(from))
        {
            contents.define(Key::name(name), Property::data(value));
        }
    }
    instance.define(
        Key::name("webContents"),
        Property::data(Value::Object(contents.clone())),
    );
    realm.register_electron_window(window.raw(), instance.clone(), contents.clone());
    // As Electron's constructor does, the app learns of the new web contents
    // and window.
    if let Some(app) = realm.electron_app.clone() {
        for (name, created) in [
            ("web-contents-created", contents),
            ("browser-window-created", instance.clone()),
        ] {
            let event = Value::Object(realm.object());
            crate::electron_window::emit(realm, &app, name, &[event, Value::Object(created)])?;
        }
    }
    Ok(())
}

/// Whether a path names a drive or share rather than a place relative to a
/// folder.
fn absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
        || path.starts_with("\\\\")
        || path.starts_with("//")
}

#[derive(Clone, Copy)]
enum WindowAction {
    Show,
    Hide,
    Close,
    Destroy,
}

/// The user agent string pages read, as Electron reports it: Chromium's,
/// with the application's name and version and Electron's version; a
/// plain Chromium one outside Electron applications.
pub(crate) fn user_agent(realm: &Realm) -> String {
    let chrome = chrome_major(realm.electron_version);
    let app = match (realm.electron_app_name, realm.electron_app_version) {
        (Some(name), Some(version)) if realm.electron_version.is_some() => {
            format!("{name}/{version} ")
        }
        _ => String::new(),
    };
    let electron = realm
        .electron_version
        .map(|version| format!("Electron/{version} "))
        .unwrap_or_default();
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) {app}Chrome/{chrome}.0.0.0 {electron}Safari/537.36"
    )
}

/// The Chromium major version an Electron release ships: two per Electron
/// major from Electron 16 (Chromium 96) on.
pub fn chrome_major(electron: Option<&str>) -> u32 {
    electron
        .and_then(|version| version.split('.').next()?.parse::<u32>().ok())
        .filter(|major| *major >= 16)
        .map_or(124, |major| 2 * major + 64)
}

#[cfg(test)]
#[path = "electron_builtin/tests.rs"]
mod tests;
