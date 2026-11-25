//! Electron's `Tray`, `Notification` and `globalShortcut`, served by the
//! host's notification window: icons in the notification area with their
//! events, tooltips, balloons and context menus, notifications shown as
//! balloons, which Windows shows as banners, and shortcuts registered with
//! the system.

use alloc::vec::Vec;
use core::ffi::c_void;

use crate::app_host::{self, BalloonKind, Icon, NotifyEvent};
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

/// What an identifier of the notification window stands for.
#[derive(Debug)]
pub(crate) enum Target {
    /// A tray icon, its context menu and its image.
    Tray {
        object: Object,
        menu: Option<Value>,
        icon: Option<Icon>,
    },
    /// A shown notification's icon.
    Notification { object: Object, icon: Option<Icon> },
    /// A notification a page of a window shows, by the page's number for
    /// it, and its own icon.
    #[cfg_attr(not(feature = "dom"), allow(dead_code))]
    Page {
        window: usize,
        notification: u32,
        icon: Option<Icon>,
    },
    /// A global shortcut's accelerator and callback.
    Shortcut {
        accelerator: String,
        callback: Value,
    },
    /// `powerMonitor` or `systemPreferences`, and the accent color last
    /// reported.
    System { object: Object, accent: Option<u32> },
}

impl Target {
    fn icon(&self) -> Option<Icon> {
        match self {
            Target::Tray { icon, .. }
            | Target::Notification { icon, .. }
            | Target::Page { icon, .. } => *icon,
            Target::Shortcut { .. } | Target::System { .. } => None,
        }
    }
}

/// A method of a class's prototype.
type Method = fn(&mut Realm, Value, &[Value]) -> crate::Completion;

fn next_id(realm: &mut Realm) -> u32 {
    realm.next_notify_id += 1;
    realm.next_notify_id
}

/// Frees an icon this module made; the application's own icon is shared.
fn release(icon: Option<Icon>) {
    if let Some(icon) = icon
        && Some(icon) != Icon::application()
    {
        icon.destroy();
    }
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn units(realm: &mut Realm, value: &Value) -> Result<Vec<u16>, Thrown> {
    Ok(match value {
        Value::Undefined | Value::Null => Vec::new(),
        value => ops::to_string(realm, value)?.units().to_vec(),
    })
}

fn option(realm: &mut Realm, options: &Value, name: &'static str) -> Result<Value, Thrown> {
    if matches!(options, Value::Object(_)) {
        ops::get_named(realm, options, name)
    } else {
        Ok(Value::Undefined)
    }
}

/// The icon of an image: a `NativeImage` or a path to a PNG file.
pub(crate) fn image_icon(realm: &mut Realm, image: &Value) -> Option<Icon> {
    let (width, height, rgba) = crate::electron_native_image::pixels(realm, image)?;
    Icon::from_rgba(width, height, &rgba)
}

fn failure(realm: &Realm, what: &str, code: u32) -> Thrown {
    realm.throw(
        ErrorKind::Error,
        &format!("{what} failed (Win32 error {code})"),
    )
}

/// The identifier of a tray or notification object.
fn id_of(realm: &Realm, object: &Value) -> Option<u32> {
    let Value::Object(object) = object else {
        return None;
    };
    realm
        .notify_targets
        .iter()
        .find(|(_, target)| match target {
            Target::Tray { object: owner, .. } | Target::Notification { object: owner, .. } => {
                owner == object
            }
            Target::Shortcut { .. } | Target::Page { .. } | Target::System { .. } => false,
        })
        .map(|(id, _)| *id)
}

/// A constructor whose instances are EventEmitters with `methods`.
fn class(
    realm: &mut Realm,
    name: &'static str,
    emitter: Option<Value>,
    methods: &[(&'static str, u32, Method)],
    construct: impl Fn(&mut Realm, Object, &[Value]) -> Result<(), Thrown> + 'static,
) -> Object {
    let prototype = realm.object();
    match &emitter {
        Some(emitter) => {
            if let Ok(Value::Object(base)) = ops::get_named(realm, emitter, "prototype") {
                prototype.set_prototype(Some(base));
            }
        }
        None => crate::electron_window::inert_listeners(realm, &prototype),
    }
    for (method, length, function) in methods {
        let function = realm.function(method, *length, false, *function);
        prototype.define(Key::name(method), Property::hidden(Value::Object(function)));
    }
    let instance_prototype = prototype.clone();
    realm.builtin_constructor(
        name,
        1,
        &prototype,
        move |realm, _, _| {
            Err(realm.throw(
                ErrorKind::Type,
                &format!("Class constructor {name} cannot be invoked without 'new'"),
            ))
        },
        move |realm, target, args| {
            let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
                Value::Object(prototype) => prototype,
                _ => instance_prototype.clone(),
            };
            let instance = Object::new(Some(prototype), crate::Kind::Ordinary);
            if let Some(emitter) = &emitter {
                ops::call(realm, emitter, Value::Object(instance.clone()), &[])?;
            }
            construct(realm, instance.clone(), args)?;
            Ok(instance)
        },
    )
}

/// `Tray`.
pub(crate) fn tray(realm: &mut Realm, emitter: Option<Value>) -> Object {
    class(
        realm,
        "Tray",
        emitter,
        &[
            ("destroy", 0, tray_destroy),
            ("isDestroyed", 0, tray_is_destroyed),
            ("setImage", 1, tray_set_image),
            ("setPressedImage", 1, nothing),
            ("setToolTip", 1, tray_set_tool_tip),
            ("setTitle", 1, nothing),
            ("getTitle", 0, |_, _, _| Ok(Value::str(""))),
            ("setIgnoreDoubleClickEvents", 1, nothing),
            ("getIgnoreDoubleClickEvents", 0, |_, _, _| {
                Ok(Value::Boolean(false))
            }),
            ("displayBalloon", 1, tray_display_balloon),
            ("removeBalloon", 0, tray_remove_balloon),
            ("focus", 0, nothing),
            ("popUpContextMenu", 2, tray_pop_up_context_menu),
            ("closeContextMenu", 0, nothing),
            ("setContextMenu", 1, tray_set_context_menu),
            ("getBounds", 0, tray_get_bounds),
        ],
        |realm, instance, args| {
            let image = arg(args, 0);
            let icon = image_icon(realm, &image);
            let id = next_id(realm);
            app_host::add_icon(id, icon, &[]).map_err(|code| {
                release(icon);
                failure(realm, "creating the tray icon", code)
            })?;
            realm.notify_targets.insert(
                id,
                Target::Tray {
                    object: instance,
                    menu: None,
                    icon,
                },
            );
            Ok(())
        },
    )
}

fn nothing(_: &mut Realm, _: Value, _: &[Value]) -> crate::Completion {
    Ok(Value::Undefined)
}

/// The identifier of a live tray, or an error as for a destroyed one.
fn live_tray(realm: &Realm, this: &Value) -> Result<u32, Thrown> {
    id_of(realm, this).ok_or_else(|| realm.throw(ErrorKind::Type, "Tray is destroyed"))
}

fn tray_destroy(realm: &mut Realm, this: Value, _: &[Value]) -> crate::Completion {
    if let Some(id) = id_of(realm, &this)
        && let Some(target) = realm.notify_targets.remove(&id)
    {
        let _ = app_host::remove_icon(id);
        release(target.icon());
    }
    Ok(Value::Undefined)
}

fn tray_is_destroyed(realm: &mut Realm, this: Value, _: &[Value]) -> crate::Completion {
    Ok(Value::Boolean(id_of(realm, &this).is_none()))
}

fn tray_set_image(realm: &mut Realm, this: Value, args: &[Value]) -> crate::Completion {
    let id = live_tray(realm, &this)?;
    let icon = image_icon(realm, &arg(args, 0));
    app_host::set_icon(id, icon).map_err(|code| {
        release(icon);
        failure(realm, "changing the tray icon", code)
    })?;
    if let Some(Target::Tray { icon: old, .. }) = realm.notify_targets.get_mut(&id) {
        release(core::mem::replace(old, icon));
    }
    Ok(Value::Undefined)
}

fn tray_set_tool_tip(realm: &mut Realm, this: Value, args: &[Value]) -> crate::Completion {
    let id = live_tray(realm, &this)?;
    let tip = units(realm, &arg(args, 0))?;
    app_host::set_tip(id, &tip).map_err(|code| failure(realm, "changing the tooltip", code))?;
    Ok(Value::Undefined)
}

/// `displayBalloon({ icon, iconType, title, content, noSound, largeIcon })`.
fn tray_display_balloon(realm: &mut Realm, this: Value, args: &[Value]) -> crate::Completion {
    let id = live_tray(realm, &this)?;
    let options = arg(args, 0);
    let title = option(realm, &options, "title")?;
    let title = units(realm, &title)?;
    let content = option(realm, &options, "content")?;
    let content = units(realm, &content)?;
    let silent = ops::to_boolean(&option(realm, &options, "noSound")?);
    let large = !matches!(option(realm, &options, "largeIcon")?, Value::Boolean(false));
    let icon = option(realm, &options, "icon")?;
    let icon_type = option(realm, &options, "iconType")?;
    let icon_type = units(realm, &icon_type)?;
    let kind = match image_icon(realm, &icon) {
        Some(icon) => BalloonKind::Custom(icon),
        None => match String::from_utf16_lossy(&icon_type).as_str() {
            "info" => BalloonKind::Info,
            "warning" => BalloonKind::Warning,
            "error" => BalloonKind::Error,
            _ => BalloonKind::None,
        },
    };
    let shown = app_host::balloon(id, &title, &content, kind, silent, large);
    if let BalloonKind::Custom(icon) = kind {
        // Windows copies the balloon's icon.
        icon.destroy();
    }
    shown.map_err(|code| failure(realm, "showing the balloon", code))?;
    Ok(Value::Undefined)
}

fn tray_remove_balloon(realm: &mut Realm, this: Value, _: &[Value]) -> crate::Completion {
    let id = live_tray(realm, &this)?;
    let _ = app_host::balloon(id, &[], &[], BalloonKind::None, true, false);
    Ok(Value::Undefined)
}

fn tray_set_context_menu(realm: &mut Realm, this: Value, args: &[Value]) -> crate::Completion {
    let id = live_tray(realm, &this)?;
    let menu = match arg(args, 0) {
        Value::Null | Value::Undefined => None,
        menu => Some(menu),
    };
    if let Some(Target::Tray { menu: current, .. }) = realm.notify_targets.get_mut(&id) {
        *current = menu;
    }
    Ok(Value::Undefined)
}

/// `popUpContextMenu([menu], [position])`: the given menu, or the one set,
/// at a screen point in device-independent pixels, or at the cursor.
fn tray_pop_up_context_menu(realm: &mut Realm, this: Value, args: &[Value]) -> crate::Completion {
    let id = live_tray(realm, &this)?;
    let menu = match arg(args, 0) {
        Value::Object(menu) => Some(Value::Object(menu)),
        _ => match realm.notify_targets.get(&id) {
            Some(Target::Tray { menu, .. }) => menu.clone(),
            _ => None,
        },
    };
    let Some(menu) = menu else {
        return Ok(Value::Undefined);
    };
    let position = arg(args, 1);
    let position = if let Value::Object(_) = position {
        let x = ops::get_named(realm, &position, "x")?;
        let y = ops::get_named(realm, &position, "y")?;
        Some((
            ops::to_number(realm, &x)? as i32,
            ops::to_number(realm, &y)? as i32,
        ))
    } else {
        None
    };
    crate::electron_menu::popup_for_icon(realm, &menu, position)?;
    Ok(Value::Undefined)
}

/// A rectangle in device-independent pixels.
fn rectangle(realm: &mut Realm, (left, top, right, bottom): (i32, i32, i32, i32)) -> Value {
    let scale = app_host::notify_scale();
    let rectangle = realm.object();
    for (name, value) in [
        ("x", left),
        ("y", top),
        ("width", right - left),
        ("height", bottom - top),
    ] {
        let value = crate::float::floor(f64::from(value) / scale + 0.5);
        rectangle.define(Key::name(name), Property::data(Value::Number(value)));
    }
    Value::Object(rectangle)
}

fn tray_get_bounds(realm: &mut Realm, this: Value, _: &[Value]) -> crate::Completion {
    let id = live_tray(realm, &this)?;
    let bounds = app_host::icon_rect(id).unwrap_or((0, 0, 0, 0));
    Ok(rectangle(realm, bounds))
}

/// `Notification`.
pub(crate) fn notification(realm: &mut Realm, emitter: Option<Value>) -> Object {
    let constructor = class(
        realm,
        "Notification",
        emitter,
        &[
            ("show", 0, notification_show),
            ("close", 0, notification_close),
        ],
        |realm, instance, args| {
            let options = arg(args, 0);
            for name in ["title", "subtitle", "body"] {
                let value = option(realm, &options, name)?;
                let value = match value {
                    Value::Undefined => Value::str(""),
                    value => Value::String(ops::to_string(realm, &value)?),
                };
                instance.define(Key::name(name), Property::data(value));
            }
            for name in ["silent", "icon", "urgency", "timeoutType"] {
                let value = option(realm, &options, name)?;
                instance.define(Key::name(name), Property::data(value));
            }
            Ok(())
        },
    );
    let supported = realm.function("isSupported", 0, false, |_, _, _| Ok(Value::Boolean(true)));
    constructor.define(
        Key::name("isSupported"),
        Property::hidden(Value::Object(supported)),
    );
    constructor
}

/// `show()`: the notification as a balloon from an icon of its own, which
/// goes when the notification is clicked or closed.
fn notification_show(realm: &mut Realm, this: Value, _: &[Value]) -> crate::Completion {
    let Value::Object(object) = this.clone() else {
        return Ok(Value::Undefined);
    };
    if let Some(id) = id_of(realm, &this) {
        close_notification(realm, id, false)?;
    }
    let title = ops::get_named(realm, &this, "title")?;
    let title = units(realm, &title)?;
    let body = ops::get_named(realm, &this, "body")?;
    let body = units(realm, &body)?;
    let silent = ops::to_boolean(&ops::get_named(realm, &this, "silent")?);
    let image = ops::get_named(realm, &this, "icon")?;
    let custom = image_icon(realm, &image);
    let icon = custom.or_else(Icon::application);
    let id = next_id(realm);
    let tip = crate::electron_builtin::app_title(realm);
    if let Err(code) = app_host::add_icon(id, icon, &tip) {
        release(custom);
        crate::electron_window::emit(
            realm,
            &object,
            "failed",
            &[Value::str(&format!(
                "the notification icon could not be added (Win32 error {code})"
            ))],
        )?;
        return Ok(Value::Undefined);
    }
    realm.notify_targets.insert(
        id,
        Target::Notification {
            object: object.clone(),
            icon: custom,
        },
    );
    let kind = icon.map_or(BalloonKind::Info, BalloonKind::Custom);
    if let Err(code) = app_host::balloon(id, &title, &body, kind, silent, true) {
        close_notification(realm, id, false)?;
        crate::electron_window::emit(
            realm,
            &object,
            "failed",
            &[Value::str(&format!(
                "the notification could not be shown (Win32 error {code})"
            ))],
        )?;
        return Ok(Value::Undefined);
    }
    let event = realm.object();
    crate::electron_window::emit(realm, &object, "show", &[Value::Object(event)])?;
    Ok(Value::Undefined)
}

/// Shows the notification a page of `window` made, from the application's
/// icon, as Electron shows a page's notifications natively; its `show`,
/// `click`, `close` and `error` events go back to the page.
#[cfg_attr(not(feature = "dom"), allow(dead_code))]
pub(crate) fn show_page(
    realm: &mut Realm,
    window: usize,
    notification: u32,
    title: &[u16],
    body: &[u16],
    silent: bool,
    pixels: Option<(u32, u32, Vec<u8>)>,
) -> Result<(), Thrown> {
    let id = next_id(realm);
    let application = Icon::application();
    let tip = crate::electron_builtin::app_title(realm);
    if app_host::add_icon(id, application, &tip).is_err() {
        realm
            .page_notification_events
            .push((window, notification, "error"));
        return Ok(());
    }
    // The notification's icon, as Chromium shows it in the toast, or the
    // application's.
    let icon = pixels.and_then(|(width, height, rgba)| {
        let (side, square) = square_icon(width, height, &rgba, PAGE_ICON_SIDE)?;
        Icon::from_rgba(side, side, &square)
    });
    realm.notify_targets.insert(
        id,
        Target::Page {
            window,
            notification,
            icon,
        },
    );
    let kind = icon
        .or(application)
        .map_or(BalloonKind::Info, BalloonKind::Custom);
    if app_host::balloon(id, title, body, kind, silent, true).is_err() {
        close_notification(realm, id, false)?;
        realm
            .page_notification_events
            .push((window, notification, "error"));
        return Ok(());
    }
    realm
        .page_notification_events
        .push((window, notification, "show"));
    Ok(())
}

/// The side of a page notification's icon, a large balloon icon's size at
/// twice the usual scale, which Windows scales down.
#[cfg_attr(not(feature = "dom"), allow(dead_code))]
const PAGE_ICON_SIDE: u32 = 64;

/// An image fit into a square of at most `limit` pixels a side, centered
/// on transparent pixels and scaled down by averaging the pixels each one
/// covers: its side and RGBA pixels.
#[cfg_attr(not(feature = "dom"), allow(dead_code))]
fn square_icon(width: u32, height: u32, rgba: &[u8], limit: u32) -> Option<(u32, Vec<u8>)> {
    if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
        return None;
    }
    let longest = width.max(height);
    let side = longest.min(limit);
    let scale = f64::from(longest) / f64::from(side);
    // The image's place in the square, in the square's pixels.
    let shown = |length: u32| ((f64::from(length) / scale + 0.5) as u32).max(1);
    let (shown_width, shown_height) = (shown(width), shown(height));
    let (left, top) = ((side - shown_width) / 2, (side - shown_height) / 2);
    let mut square = alloc::vec![0u8; side as usize * side as usize * 4];
    for y in 0..shown_height {
        for x in 0..shown_width {
            // The source pixels this one covers, averaged with their alpha.
            let x0 = (f64::from(x) * scale) as u32;
            let y0 = (f64::from(y) * scale) as u32;
            let x1 = ((f64::from(x + 1) * scale) as u32).clamp(x0 + 1, width);
            let y1 = ((f64::from(y + 1) * scale) as u32).clamp(y0 + 1, height);
            let mut sum = [0u64; 4];
            for source_y in y0..y1 {
                for source_x in x0..x1 {
                    let at = (source_y as usize * width as usize + source_x as usize) * 4;
                    let alpha = u64::from(rgba[at + 3]);
                    for channel in 0..3 {
                        sum[channel] += u64::from(rgba[at + channel]) * alpha;
                    }
                    sum[3] += alpha;
                }
            }
            let count = u64::from((x1 - x0) * (y1 - y0));
            let at = (((top + y) * side + left + x) * 4) as usize;
            for channel in 0..3 {
                // Transparent pixels stay black.
                if let Some(value) = sum[channel].checked_div(sum[3]) {
                    square[at + channel] = value as u8;
                }
            }
            square[at + 3] = (sum[3] / count) as u8;
        }
    }
    Some((side, square))
}

/// Closes the notification a page of `window` made, if it shows.
#[cfg_attr(not(feature = "dom"), allow(dead_code))]
pub(crate) fn close_page(
    realm: &mut Realm,
    window: usize,
    notification: u32,
) -> Result<(), Thrown> {
    let id = realm.notify_targets.iter().find_map(|(id, target)| {
        matches!(target, Target::Page { window: shown, notification: number, .. }
            if *shown == window && *number == notification)
        .then_some(*id)
    });
    match id {
        Some(id) => close_notification(realm, id, true),
        None => Ok(()),
    }
}

fn notification_close(realm: &mut Realm, this: Value, _: &[Value]) -> crate::Completion {
    if let Some(id) = id_of(realm, &this) {
        close_notification(realm, id, true)?;
    }
    Ok(Value::Undefined)
}

/// Removes a notification's icon, and with `announce` emits `close`.
fn close_notification(realm: &mut Realm, id: u32, announce: bool) -> Result<(), Thrown> {
    let Some(target) = realm.notify_targets.remove(&id) else {
        return Ok(());
    };
    let _ = app_host::remove_icon(id);
    release(target.icon());
    match target {
        Target::Notification { object, .. } if announce => {
            let event = realm.object();
            crate::electron_window::emit(realm, &object, "close", &[Value::Object(event)])?;
        }
        Target::Page {
            window,
            notification,
            ..
        } if announce => realm
            .page_notification_events
            .push((window, notification, "close")),
        _ => {}
    }
    Ok(())
}

/// `globalShortcut`.
pub(crate) fn global_shortcut(realm: &mut Realm) -> Object {
    let shortcut = realm.object();
    let methods: [(&str, u32, Method); 5] = [
        ("register", 2, shortcut_register),
        ("registerAll", 2, shortcut_register_all),
        ("isRegistered", 1, shortcut_is_registered),
        ("unregister", 1, shortcut_unregister),
        ("unregisterAll", 0, shortcut_unregister_all),
    ];
    for (name, length, function) in methods {
        let function = realm.function(name, length, false, function);
        shortcut.define(Key::name(name), Property::data(Value::Object(function)));
    }
    shortcut
}

fn shortcut_id(realm: &Realm, accelerator: &str) -> Option<u32> {
    realm
        .notify_targets
        .iter()
        .find(|(_, target)| {
            matches!(target, Target::Shortcut { accelerator: registered, .. }
                if registered.eq_ignore_ascii_case(accelerator))
        })
        .map(|(id, _)| *id)
}

fn register(realm: &mut Realm, accelerator: &Value, callback: &Value) -> Result<bool, Thrown> {
    let accelerator = ops::to_string(realm, accelerator)?;
    let text = accelerator.to_rust_lossy();
    if !crate::array_builtin::is_callable(callback) {
        return Err(realm.throw(ErrorKind::Type, "callback must be a function"));
    }
    if shortcut_id(realm, &text).is_some() {
        return Ok(false);
    }
    let id = next_id(realm);
    if !app_host::register_hotkey(id, accelerator.units()) {
        return Ok(false);
    }
    realm.notify_targets.insert(
        id,
        Target::Shortcut {
            accelerator: text,
            callback: callback.clone(),
        },
    );
    Ok(true)
}

fn shortcut_register(realm: &mut Realm, _: Value, args: &[Value]) -> crate::Completion {
    register(realm, &arg(args, 0), &arg(args, 1)).map(Value::Boolean)
}

fn shortcut_register_all(realm: &mut Realm, _: Value, args: &[Value]) -> crate::Completion {
    let accelerators = arg(args, 0);
    let callback = arg(args, 1);
    let length = ops::get_named(realm, &accelerators, "length")?;
    let length = ops::to_number(realm, &length)? as u32;
    for index in 0..length {
        let accelerator = ops::get(realm, &accelerators, &Key::Index(index))?;
        register(realm, &accelerator, &callback)?;
    }
    Ok(Value::Undefined)
}

fn shortcut_is_registered(realm: &mut Realm, _: Value, args: &[Value]) -> crate::Completion {
    let accelerator = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
    Ok(Value::Boolean(shortcut_id(realm, &accelerator).is_some()))
}

fn shortcut_unregister(realm: &mut Realm, _: Value, args: &[Value]) -> crate::Completion {
    let accelerator = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
    if let Some(id) = shortcut_id(realm, &accelerator) {
        app_host::unregister_hotkey(id);
        realm.notify_targets.remove(&id);
    }
    Ok(Value::Undefined)
}

fn shortcut_unregister_all(realm: &mut Realm, _: Value, _: &[Value]) -> crate::Completion {
    let ids: Vec<u32> = realm
        .notify_targets
        .iter()
        .filter(|(_, target)| matches!(target, Target::Shortcut { .. }))
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        app_host::unregister_hotkey(id);
        realm.notify_targets.remove(&id);
    }
    Ok(Value::Undefined)
}

/// The keyboard state of a tray event, as Electron's `KeyboardEvent`.
fn keyboard_event(realm: &mut Realm) -> Value {
    let (shift, control, alt, windows) = app_host::held_modifiers();
    let event = realm.object();
    for (name, held) in [
        ("altKey", alt),
        ("shiftKey", shift),
        ("ctrlKey", control),
        ("metaKey", windows),
    ] {
        event.define(Key::name(name), Property::data(Value::Boolean(held)));
    }
    Value::Object(event)
}

/// A screen point in device-independent pixels.
fn point(realm: &mut Realm, (x, y): (i32, i32)) -> Value {
    let scale = app_host::notify_scale();
    let point = realm.object();
    for (name, value) in [("x", x), ("y", y)] {
        let value = crate::float::floor(f64::from(value) / scale + 0.5);
        point.define(Key::name(name), Property::data(Value::Number(value)));
    }
    Value::Object(point)
}

/// Emits a system event on `powerMonitor` or `systemPreferences`, with an
/// event object first, as Electron's native emitters give it.
fn system_event(
    realm: &mut Realm,
    target: u32,
    name: &str,
    argument: Option<Value>,
) -> Result<(), Thrown> {
    let Some(Target::System { object, .. }) = realm.notify_targets.get(&target) else {
        return Ok(());
    };
    let object = object.clone();
    let mut args = Vec::from([Value::Object(realm.object())]);
    args.extend(argument);
    crate::electron_window::emit(realm, &object, name, &args).map(drop)
}

/// The host's callback for icon events and shortcut presses.
///
/// # Safety
/// `context` is the main realm, live for the message loop, as the generated
/// entry registers it.
pub unsafe extern "system" fn notify_event(context: *mut c_void, event: NotifyEvent, id: u32) {
    if context.is_null() {
        return;
    }
    // SAFETY: guaranteed by the caller.
    let realm = unsafe { &mut *context.cast::<Realm>() };
    if let Err(thrown) = dispatch(realm, event, id) {
        realm.host_errors.push(thrown);
    }
}

fn dispatch(realm: &mut Realm, event: NotifyEvent, id: u32) -> Result<(), Thrown> {
    if let Some(name) = match event {
        NotifyEvent::Suspend => Some("suspend"),
        NotifyEvent::Resume => Some("resume"),
        NotifyEvent::OnAc => Some("on-ac"),
        NotifyEvent::OnBattery => Some("on-battery"),
        NotifyEvent::LockScreen => Some("lock-screen"),
        NotifyEvent::UnlockScreen => Some("unlock-screen"),
        _ => None,
    } {
        return system_event(realm, crate::electron_system::POWER_MONITOR, name, None);
    }
    match event {
        NotifyEvent::AccentColor => {
            let target = crate::electron_system::SYSTEM_PREFERENCES;
            // Only a new color counts.
            let changed = match realm.notify_targets.get_mut(&target) {
                Some(Target::System { accent, .. }) if *accent != Some(id) => {
                    *accent = Some(id);
                    true
                }
                _ => false,
            };
            if !changed {
                return Ok(());
            }
            let color = Value::str(&crate::electron_system::accent_hex(id));
            return system_event(realm, target, "accent-color-changed", Some(color));
        }
        NotifyEvent::SystemColors => {
            let target = crate::electron_system::SYSTEM_PREFERENCES;
            return system_event(realm, target, "color-changed", None);
        }
        _ => {}
    }
    match realm.notify_targets.get(&id) {
        Some(Target::Shortcut { callback, .. }) => {
            let callback = callback.clone();
            ops::call(realm, &callback, Value::Undefined, &[])?;
        }
        Some(Target::Notification { object, .. }) => {
            let object = object.clone();
            match event {
                NotifyEvent::BalloonClick => {
                    let event = realm.object();
                    crate::electron_window::emit(realm, &object, "click", &[Value::Object(event)])?;
                    close_notification(realm, id, true)?;
                }
                NotifyEvent::BalloonClosed => close_notification(realm, id, true)?,
                _ => {}
            }
        }
        Some(&Target::Page {
            window,
            notification,
            ..
        }) => match event {
            NotifyEvent::BalloonClick => {
                realm
                    .page_notification_events
                    .push((window, notification, "click"));
                close_notification(realm, id, true)?;
            }
            NotifyEvent::BalloonClosed => close_notification(realm, id, true)?,
            _ => {}
        },
        Some(Target::System { .. }) => {}
        Some(Target::Tray { object, menu, .. }) => {
            let (object, menu) = (object.clone(), menu.clone());
            let bounds = app_host::icon_rect(id).unwrap_or((0, 0, 0, 0));
            let bounds = rectangle(realm, bounds);
            let position = app_host::cursor();
            let position = point(realm, position);
            let keys = keyboard_event(realm);
            let (name, args) = match event {
                NotifyEvent::Click => ("click", Vec::from([keys, bounds, position])),
                NotifyEvent::RightClick => ("right-click", Vec::from([keys, bounds])),
                NotifyEvent::DoubleClick => ("double-click", Vec::from([keys, bounds])),
                NotifyEvent::MiddleClick => ("middle-click", Vec::from([keys, bounds])),
                NotifyEvent::MouseMove => ("mouse-move", Vec::from([keys, position])),
                NotifyEvent::BalloonShow => ("balloon-show", Vec::new()),
                NotifyEvent::BalloonClick => ("balloon-click", Vec::new()),
                NotifyEvent::BalloonClosed => ("balloon-closed", Vec::new()),
                // Shortcuts and system events go elsewhere.
                _ => return Ok(()),
            };
            crate::electron_window::emit(realm, &object, name, &args)?;
            // The menu set with setContextMenu opens on a right click.
            if event == NotifyEvent::RightClick
                && let Some(menu) = menu
            {
                crate::electron_menu::popup_for_icon(realm, &menu, None)?;
            }
        }
        None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
