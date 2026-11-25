//! Native Windows implementation of the Electron menu surface used by
//! MarkText. Menu trees stay as ordinary compiled values; Win32 only owns the
//! visible menu and returns command identifiers to the live native Realm.

use alloc::vec::Vec;
use core::ffi::c_void;

use crate::array_builtin::is_array;
use crate::prelude::*;
use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Slot, Thrown, Value, app_host, ops};

/// The `Menu` and `MenuItem` exports for the generated Electron module.
pub fn module(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let menu_prototype = realm.object();
    let menu_item_prototype = realm.object();
    let item_prototype = menu_item_prototype.clone();
    let menu_item_constructor = realm.builtin_constructor(
        "MenuItem",
        1,
        &menu_item_prototype,
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "MenuItem must be constructed with new")),
        move |realm, _, args| {
            let item = Object::new(Some(item_prototype.clone()), Kind::Ordinary);
            initialize_menu_item(
                realm,
                &item,
                args.first().cloned().unwrap_or(Value::Undefined),
            )?;
            Ok(item)
        },
    );
    let instance_prototype = menu_prototype.clone();
    let menu_constructor = realm.builtin_constructor(
        "Menu",
        0,
        &menu_prototype,
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "Menu must be constructed with new")),
        move |realm, _, _| {
            let menu = Object::new(Some(instance_prototype.clone()), Kind::Ordinary);
            menu.define(
                Key::name("items"),
                Property::data(Value::Object(realm.array(Vec::new()))),
            );
            Ok(menu)
        },
    );

    menu_prototype.define(
        Key::name("append"),
        Property::hidden(Value::Object(realm.function(
            "append",
            1,
            false,
            |realm, this, args| {
                let item = args.first().cloned().unwrap_or(Value::Undefined);
                if !is_menu_item(realm, &item) {
                    return Err(realm.throw(ErrorKind::Type, "Menu.append expects a MenuItem"));
                }
                let items = ops::get_named(realm, &this, "items")?;
                let length = array_length(realm, &items)?;
                let Value::Object(items) = items else {
                    return Err(realm.throw(ErrorKind::Type, "Menu.items must be an array"));
                };
                items.define(Key::Index(length), Property::data(item));
                ops::set(
                    realm,
                    &Value::Object(items),
                    &Key::name("length"),
                    Value::Number(f64::from(length + 1)),
                )?;
                Ok(menus_changed(realm))
            },
        ))),
    );
    menu_prototype.define(
        Key::name("insert"),
        Property::hidden(Value::Object(realm.function(
            "insert",
            2,
            false,
            |realm, this, args| {
                let index = integer_argument(args.first(), 0);
                let item = args.get(1).cloned().unwrap_or(Value::Undefined);
                if !is_menu_item(realm, &item) {
                    return Err(realm.throw(ErrorKind::Type, "Menu.insert expects a MenuItem"));
                }
                let items = ops::get_named(realm, &this, "items")?;
                let length = array_length(realm, &items)?;
                let index = index.min(length);
                for position in (index..length).rev() {
                    let value = ops::get(realm, &items, &Key::Index(position))?;
                    if let Value::Object(items) = &items {
                        items.define(Key::Index(position + 1), Property::data(value));
                    }
                }
                if let Value::Object(items) = &items {
                    items.define(Key::Index(index), Property::data(item));
                }
                ops::set(
                    realm,
                    &items,
                    &Key::name("length"),
                    Value::Number(f64::from(length + 1)),
                )?;
                Ok(menus_changed(realm))
            },
        ))),
    );
    menu_prototype.define(
        Key::name("remove"),
        Property::hidden(Value::Object(realm.function(
            "remove",
            1,
            false,
            |realm, this, args| {
                let id = args.first().cloned().unwrap_or(Value::Undefined);
                let items = ops::get_named(realm, &this, "items")?;
                let length = array_length(realm, &items)?;
                let mut found = None;
                for index in 0..length {
                    let item = ops::get(realm, &items, &Key::Index(index))?;
                    let item_id = ops::get_named(realm, &item, "id")?;
                    if same_id(&item_id, &id) {
                        found = Some(index);
                        break;
                    }
                }
                if let Some(index) = found {
                    remove_at(realm, &items, index, length)?;
                    menus_changed(realm);
                }
                Ok(Value::Undefined)
            },
        ))),
    );
    menu_prototype.define(
        Key::name("removeAt"),
        Property::hidden(Value::Object(realm.function(
            "removeAt",
            1,
            false,
            |realm, this, args| {
                let index = integer_argument(args.first(), 0);
                let items = ops::get_named(realm, &this, "items")?;
                let length = array_length(realm, &items)?;
                if index < length {
                    remove_at(realm, &items, index, length)?;
                    menus_changed(realm);
                }
                Ok(Value::Undefined)
            },
        ))),
    );
    menu_prototype.define(
        Key::name("getMenuItemById"),
        Property::hidden(Value::Object(realm.function(
            "getMenuItemById",
            1,
            false,
            |realm, this, args| {
                let id = args.first().cloned().unwrap_or(Value::Undefined);
                let items = ops::get_named(realm, &this, "items")?;
                find_item(realm, &items, &id)
                    .map(|item| item.map(Value::Object).unwrap_or(Value::Undefined))
            },
        ))),
    );
    menu_prototype.define(
        Key::name("popup"),
        Property::hidden(Value::Object(realm.function(
            "popup",
            1,
            false,
            |realm, this, args| {
                let options = args.first().cloned().unwrap_or(Value::Undefined);
                let owner = window_option(realm, &options)?;
                // Without both coordinates the menu opens at the cursor.
                let given = |realm: &mut Realm, name: &str| -> Result<bool, Thrown> {
                    Ok(matches!(options, Value::Object(_))
                        && !matches!(
                            ops::get(realm, &options, &Key::name(name))?,
                            Value::Undefined
                        ))
                };
                let position = if given(realm, "x")? && given(realm, "y")? {
                    Some((
                        integer_property(realm, &options, "x", 0)?,
                        integer_property(realm, &options, "y", 0)?,
                    ))
                } else {
                    None
                };
                let items = ops::get_named(realm, &this, "items")?;
                let mut native = native_items(realm, &items)?;
                without_bracketed_mnemonics(&mut native);
                let chosen = app_host::popup_menu(owner, &native, position);
                crate::platform::trace_ipc(|| {
                    alloc::format!(
                        "menu popup of {} items at {position:?} chose {chosen:?}",
                        native.len()
                    )
                });
                if let Some(command) = chosen.map_err(|code| {
                    realm.throw(
                        ErrorKind::Error,
                        &format!("native menu failed (Win32 error {code})"),
                    )
                })? {
                    dispatch(
                        realm,
                        command,
                        &items,
                        owner.map(app_host::Window::raw),
                        false,
                    )?;
                }
                Ok(Value::Undefined)
            },
        ))),
    );

    menu_constructor.define(
        Key::name("buildFromTemplate"),
        Property::hidden(Value::Object(realm.function(
            "buildFromTemplate",
            1,
            false,
            move |realm, _, args| {
                let template = args.first().cloned().unwrap_or(Value::Undefined);
                if !is_array(realm, &template)? {
                    return Err(
                        realm.throw(ErrorKind::Type, "Menu.buildFromTemplate expects an array")
                    );
                }
                let menu = realm.object();
                let length = array_length(realm, &template)?;
                let items = realm.array(Vec::new());
                for index in 0..length {
                    let options = ops::get(realm, &template, &Key::Index(index))?;
                    let item = realm.object();
                    initialize_menu_item(realm, &item, options)?;
                    items.define(Key::Index(index), Property::data(Value::Object(item)));
                }
                ops::set(
                    realm,
                    &Value::Object(items.clone()),
                    &Key::name("length"),
                    Value::Number(f64::from(length)),
                )?;
                menu.define(Key::name("items"), Property::data(Value::Object(items)));
                attach_menu_prototype(&menu, &menu_prototype);
                Ok(Value::Object(menu))
            },
        ))),
    );
    menu_constructor.define(
        Key::name("setApplicationMenu"),
        Property::hidden(Value::Object(realm.function(
            "setApplicationMenu",
            1,
            false,
            |realm, _, args| {
                let menu = args.first().cloned().unwrap_or(Value::Null);
                let menu_object = match menu {
                    Value::Object(menu) => Some(menu),
                    Value::Null | Value::Undefined => None,
                    _ => {
                        return Err(
                            realm.throw(ErrorKind::Type, "application menu must be a Menu or null")
                        );
                    }
                };
                realm.electron_application_menu = menu_object.clone();
                realm.electron_installed_menus.clear();
                if let Some(menu) = menu_object {
                    let items = ops::get_named(realm, &Value::Object(menu), "items")?;
                    let native = native_items(realm, &items)?;
                    app_host::set_application_menu(&native).map_err(|code| {
                        realm.throw(
                            ErrorKind::Error,
                            &format!("native menu failed (Win32 error {code})"),
                        )
                    })?;
                } else {
                    app_host::clear_application_menu().map_err(|code| {
                        realm.throw(
                            ErrorKind::Error,
                            &format!("native menu failed (Win32 error {code})"),
                        )
                    })?;
                }
                Ok(Value::Undefined)
            },
        ))),
    );
    menu_constructor.define(
        Key::name("getApplicationMenu"),
        Property::hidden(Value::Object(realm.function(
            "getApplicationMenu",
            0,
            false,
            |realm, _, _| {
                Ok(realm
                    .electron_application_menu
                    .clone()
                    .map(Value::Object)
                    .unwrap_or(Value::Null))
            },
        ))),
    );
    menu_constructor.define(
        Key::name("sendActionToFirstResponder"),
        Property::hidden(Value::Object(realm.function(
            "sendActionToFirstResponder",
            1,
            false,
            |_, _, _| {
                // This is an AppKit-only API. On Windows Electron does not route it
                // to a Cocoa responder, so it has no platform action to perform.
                Ok(Value::Undefined)
            },
        ))),
    );

    exports.define(
        Key::name("Menu"),
        Property::data(Value::Object(menu_constructor)),
    );
    exports.define(
        Key::name("MenuItem"),
        Property::data(Value::Object(menu_item_constructor)),
    );
    exports
}

/// Installs the retained application menu on a newly created BrowserWindow.
pub fn attach_application_menu(realm: &mut Realm, window: app_host::Window) -> Result<(), Thrown> {
    realm.electron_installed_menus.clear();
    if let Some(menu) = realm.electron_application_menu.clone() {
        let items = ops::get_named(realm, &Value::Object(menu), "items")?;
        let native = native_items(realm, &items)?;
        app_host::set_window_menu(window, &native).map_err(|code| {
            realm.throw(
                ErrorKind::Error,
                &format!("native menu failed (Win32 error {code})"),
            )
        })?;
    }
    Ok(())
}

/// Sets or removes a BrowserWindow-specific Electron menu.
pub fn set_window_menu(
    realm: &mut Realm,
    window: app_host::Window,
    menu: Value,
) -> Result<(), Thrown> {
    realm.electron_installed_menus.clear();
    let Some(menu) = (match menu {
        Value::Object(menu) => Some(menu),
        Value::Null | Value::Undefined => None,
        _ => return Err(realm.throw(ErrorKind::Type, "window menu must be a Menu or null")),
    }) else {
        app_host::clear_window_menu(window).map_err(|code| {
            realm.throw(
                ErrorKind::Error,
                &format!("native menu failed (Win32 error {code})"),
            )
        })?;
        forget_window_menu(realm, window);
        return Ok(());
    };
    let items = ops::get_named(realm, &Value::Object(menu.clone()), "items")?;
    let native = native_items(realm, &items)?;
    app_host::set_window_menu(window, &native).map_err(|code| {
        realm.throw(
            ErrorKind::Error,
            &format!("native menu failed (Win32 error {code})"),
        )
    })?;
    if let Some((_, existing)) = realm
        .electron_window_menus
        .iter_mut()
        .find(|(raw, _)| *raw == window.raw())
    {
        *existing = menu;
    } else {
        realm.electron_window_menus.push((window.raw(), menu));
    }
    Ok(())
}

/// Removes a destroyed BrowserWindow's retained menu tree.
pub fn forget_window_menu(realm: &mut Realm, window: app_host::Window) {
    realm
        .electron_window_menus
        .retain(|(raw, _)| *raw != window.raw());
}

/// Entry called by the no-std Win32 window procedure for `WM_COMMAND`.
///
/// # Safety
///
/// `context` must be null or point to the live Realm registered by the
/// generated application before its UI message loop.
pub unsafe extern "system" fn dispatch_menu_command(
    context: *mut c_void,
    window: usize,
    command: u16,
    accelerator: bool,
) {
    if context.is_null() {
        return;
    }
    // SAFETY: the generated entry registers its stack Realm immediately before
    // the UI message loop, after all synchronous Realm borrows have ended.
    let realm = unsafe { &mut *(context.cast::<Realm>()) };
    if command == 0 {
        forget_window_menu(realm, app_host::Window::from_raw(window));
        return;
    }
    let menu = realm
        .electron_window_menus
        .iter()
        .find(|(raw, _)| *raw == window)
        .map(|(_, menu)| menu.clone())
        .or_else(|| realm.electron_application_menu.clone());
    let Some(menu) = menu else {
        return;
    };
    let result = ops::get_named(realm, &Value::Object(menu), "items")
        .and_then(|items| dispatch(realm, command, &items, Some(window), accelerator));
    if result.is_err() {
        // An exception from a native window-procedure callback cannot unwind
        // through Win32. Terminate through the normal host loop with failure
        // rather than silently dropping the thrown value.
        app_host::request_quit(1);
    }
}

fn initialize_menu_item(realm: &mut Realm, item: &Object, options: Value) -> Result<(), Thrown> {
    let options = match options {
        Value::Object(options) => options,
        Value::String(label) => {
            let options = realm.object();
            options.define(Key::name("label"), Property::data(Value::String(label)));
            options
        }
        Value::Undefined | Value::Null => realm.object(),
        _ => return Err(realm.throw(ErrorKind::Type, "MenuItem options must be an object")),
    };
    let command = realm.electron_next_menu_id.max(1);
    realm.electron_next_menu_id = command.wrapping_add(1).max(1);
    item.set_private(
        &realm.electron_menu_command_key,
        Property::hidden(Value::Number(f64::from(command))),
    );
    let backing = realm.object();
    for (name, default) in [
        ("id", Value::Undefined),
        ("label", Value::str("")),
        ("type", Value::str("normal")),
        ("enabled", Value::Boolean(true)),
        ("checked", Value::Boolean(false)),
        ("visible", Value::Boolean(true)),
        ("accelerator", Value::Undefined),
        ("role", Value::Undefined),
        ("click", Value::Undefined),
        ("submenu", Value::Undefined),
    ] {
        let mut value = ops::get(realm, &Value::Object(options.clone()), &Key::name(name))?;
        if matches!(value, Value::Undefined) {
            value = default;
        }
        if name == "submenu" && is_array(realm, &value)? {
            let submenu = build_menu(realm, &value)?;
            value = Value::Object(submenu);
        }
        let key = Key::name(name);
        backing.define(key.clone(), Property::data(value));
        let getter_backing = backing.clone();
        let getter_key = key.clone();
        let getter = realm.function(name, 0, false, move |realm, _, _| {
            ops::get(realm, &Value::Object(getter_backing.clone()), &getter_key)
        });
        let setter_backing = backing.clone();
        let setter_key = key.clone();
        // Pages set the same state over and over, as MarkText does on each
        // keystroke; only a different value, of what menus show, changes them.
        let shown = name != "click";
        let setter = realm.function(name, 1, false, move |realm, _, args| {
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            let backing = Value::Object(setter_backing.clone());
            let old = ops::get(realm, &backing, &setter_key)?;
            ops::set(realm, &backing, &setter_key, value.clone())?;
            if shown && !crate::object::same_value(&old, &value) {
                menus_changed(realm);
            }
            Ok(Value::Undefined)
        });
        item.define(
            key,
            Property {
                slot: Slot::Accessor {
                    get: Some(getter),
                    set: Some(setter),
                },
                enumerable: true,
                configurable: true,
            },
        );
    }
    Ok(())
}

fn build_menu(realm: &mut Realm, template: &Value) -> Result<Object, Thrown> {
    let menu = realm.object();
    let length = array_length(realm, template)?;
    let array = realm.array(Vec::new());
    for index in 0..length {
        let options = ops::get(realm, template, &Key::Index(index))?;
        let item = realm.object();
        initialize_menu_item(realm, &item, options)?;
        array.define(Key::Index(index), Property::data(Value::Object(item)));
    }
    ops::set(
        realm,
        &Value::Object(array.clone()),
        &Key::name("length"),
        Value::Number(f64::from(length)),
    )?;
    menu.define(Key::name("items"), Property::data(Value::Object(array)));
    Ok(menu)
}

fn attach_menu_prototype(menu: &Object, prototype: &Object) {
    menu.set_prototype(Some(prototype.clone()));
}

/// Opens a menu as a notification icon's context menu, at a screen point in
/// device-independent pixels or at the cursor, and runs the chosen item.
pub(crate) fn popup_for_icon(
    realm: &mut Realm,
    menu: &Value,
    position: Option<(i32, i32)>,
) -> Result<(), Thrown> {
    let items = ops::get_named(realm, menu, "items")?;
    let mut native = native_items(realm, &items)?;
    without_bracketed_mnemonics(&mut native);
    let chosen = app_host::icon_menu(&native, position).map_err(|code| {
        realm.throw(
            ErrorKind::Error,
            &format!("native menu failed (Win32 error {code})"),
        )
    })?;
    if let Some(command) = chosen {
        dispatch(realm, command, &items, None, false)?;
    }
    Ok(())
}

fn native_items(realm: &mut Realm, items: &Value) -> Result<Vec<app_host::NativeMenuItem>, Thrown> {
    let length = array_length(realm, items)?;
    let mut output = Vec::with_capacity(length as usize);
    for index in 0..length {
        let item = ops::get(realm, items, &Key::Index(index))?;
        if !ops::to_boolean(&ops::get_named(realm, &item, "visible")?) {
            continue;
        }
        let kind = ops::get_named(realm, &item, "type")?;
        let separator = matches!(kind, Value::String(ref value) if value.units() == "separator".encode_utf16().collect::<Vec<_>>());
        let radio = matches!(kind, Value::String(ref value) if value.units() == "radio".encode_utf16().collect::<Vec<_>>());
        let accelerator = match ops::get_named(realm, &item, "accelerator")? {
            Value::String(accelerator) => Some(accelerator.units().to_vec()),
            _ => None,
        };
        let submenu = ops::get_named(realm, &item, "submenu")?;
        let children = if matches!(submenu, Value::Object(_)) {
            let child_items = ops::get_named(realm, &submenu, "items")?;
            native_items(realm, &child_items)?
        } else {
            Vec::new()
        };
        let role = match ops::get_named(realm, &item, "role")? {
            Value::String(role) => role.to_rust_lossy(),
            _ => String::new(),
        };
        let (role_label, role_accelerator) = role_defaults(&role);
        let label = ops::get_named(realm, &item, "label")?;
        let mut label = ops::to_string(realm, &label)?.units().to_vec();
        // Roles name items the page leaves unnamed.
        if label.is_empty() {
            label = role_label.encode_utf16().collect();
        }
        label.push(0);
        let command = private_command(realm, &item);
        // A role's accelerator shows beside it, as Electron shows it.
        let shortcut = accelerator
            .as_deref()
            .map(String::from_utf16_lossy)
            .or_else(|| (!role_accelerator.is_empty()).then(|| role_accelerator.to_string()))
            .map(|accelerator| {
                shortcut_text(&accelerator, &crate::platform::user_locale())
                    .encode_utf16()
                    .collect()
            });
        output.push(app_host::NativeMenuItem {
            id: command.unwrap_or(0),
            label,
            separator,
            submenu: matches!(submenu, Value::Object(_)),
            enabled: ops::to_boolean(&ops::get_named(realm, &item, "enabled")?),
            checked: ops::to_boolean(&ops::get_named(realm, &item, "checked")?),
            radio,
            accelerator,
            shortcut,
            children,
        });
    }
    Ok(output)
}

/// Chooses a menu item as Electron does: checkbox items flip, radio items
/// become their group's checked item, then `click(item, window, event)`
/// runs, or the item's role.
fn dispatch(
    realm: &mut Realm,
    command: u16,
    items: &Value,
    window: Option<usize>,
    accelerator: bool,
) -> Result<(), Thrown> {
    let Some((siblings, index)) = find_native_command(realm, items, command)? else {
        return Ok(());
    };
    let item = match ops::get(realm, &siblings, &Key::Index(index))? {
        Value::Object(item) => item,
        _ => return Ok(()),
    };
    let value = Value::Object(item.clone());
    let kind = ops::get_named(realm, &value, "type")?;
    let kind = ops::to_string(realm, &kind)?.to_rust_lossy();
    if kind == "checkbox" {
        let checked = ops::to_boolean(&ops::get_named(realm, &value, "checked")?);
        ops::set(
            realm,
            &value,
            &Key::name("checked"),
            Value::Boolean(!checked),
        )?;
    } else if kind == "radio" {
        check_radio(realm, &siblings, index)?;
    }
    let click = ops::get_named(realm, &value, "click")?;
    if matches!(click, Value::Object(_)) {
        let event = realm.object();
        event.define(
            Key::name("defaultPrevented"),
            Property::data(Value::Boolean(false)),
        );
        let (shift, ctrl, alt, meta) = app_host::held_modifiers();
        for (name, flag) in [
            ("triggeredByAccelerator", accelerator),
            ("shiftKey", shift),
            ("ctrlKey", ctrl),
            ("altKey", alt),
            ("metaKey", meta),
        ] {
            event.define(Key::name(name), Property::data(Value::Boolean(flag)));
        }
        let weak_event = event.downgrade();
        let prevent = realm.function("preventDefault", 0, false, move |realm, _, _| {
            if let Some(event) = weak_event.upgrade() {
                ops::set(
                    realm,
                    &Value::Object(event),
                    &Key::name("defaultPrevented"),
                    Value::Boolean(true),
                )?;
            }
            Ok(Value::Undefined)
        });
        event.define(
            Key::name("preventDefault"),
            Property::hidden(Value::Object(prevent)),
        );
        let window = window
            .and_then(|window| realm.electron_window_objects(window))
            .map_or(Value::Undefined, |(instance, _)| Value::Object(instance));
        ops::call(
            realm,
            &click,
            value.clone(),
            &[value, window, Value::Object(event)],
        )?;
    } else if let Value::String(role) = ops::get_named(realm, &Value::Object(item), "role")? {
        app_host::send_edit_action(&role.to_rust_lossy());
    }
    Ok(())
}

fn is_menu_item(realm: &Realm, item: &Value) -> bool {
    private_command(realm, item).is_some()
}

fn private_command(realm: &Realm, item: &Value) -> Option<u16> {
    let Value::Object(item) = item else {
        return None;
    };
    match item
        .private_property(&realm.electron_menu_command_key)?
        .slot
    {
        Slot::Data {
            value: Value::Number(command),
            ..
        } if command >= 1.0 && command <= f64::from(u16::MAX) => Some(command as u16),
        _ => None,
    }
}

fn array_length(realm: &mut Realm, value: &Value) -> Result<u32, Thrown> {
    if matches!(value, Value::Undefined | Value::Null) {
        return Ok(0);
    }
    match ops::get_named(realm, value, "length")? {
        Value::Number(length) if length.is_finite() && length > 0.0 => {
            Ok(length.min(u32::MAX as f64) as u32)
        }
        _ => Ok(0),
    }
}

fn integer_argument(value: Option<&Value>, default: u32) -> u32 {
    match value {
        Some(Value::Number(number)) if number.is_finite() && *number >= 0.0 => *number as u32,
        _ => default,
    }
}

fn integer_property(
    realm: &mut Realm,
    options: &Value,
    name: &str,
    default: i32,
) -> Result<i32, Thrown> {
    let value = ops::get(realm, options, &Key::name(name))?;
    Ok(match value {
        Value::Number(number)
            if number.is_finite() && number >= i32::MIN as f64 && number <= i32::MAX as f64 =>
        {
            number as i32
        }
        _ => default,
    })
}

fn window_option(realm: &mut Realm, options: &Value) -> Result<Option<app_host::Window>, Thrown> {
    if !matches!(options, Value::Object(_)) {
        return Ok(None);
    }
    let window = ops::get_named(realm, options, "window")?;
    // BrowserWindow keeps its handle under a private symbol key.
    if let Value::Object(window) = window
        && let Some(Property {
            slot:
                Slot::Data {
                    value: Value::Number(raw),
                    ..
                },
            ..
        }) = window.own_property(&Key::Symbol(realm.electron_window_key.clone()))
        && raw > 0.0
    {
        return Ok(Some(app_host::Window::from_raw(raw as usize)));
    }
    Ok(None)
}

fn remove_at(realm: &mut Realm, items: &Value, index: u32, length: u32) -> Result<(), Thrown> {
    for position in index..length - 1 {
        let value = ops::get(realm, items, &Key::Index(position + 1))?;
        if let Value::Object(items) = items {
            items.define(Key::Index(position), Property::data(value));
        }
    }
    if let Value::Object(items) = items {
        ops::set(
            realm,
            &Value::Object(items.clone()),
            &Key::name("length"),
            Value::Number(f64::from(length - 1)),
        )?;
    }
    Ok(())
}

fn find_item(realm: &mut Realm, items: &Value, id: &Value) -> Result<Option<Object>, Thrown> {
    let length = array_length(realm, items)?;
    for index in 0..length {
        let item = ops::get(realm, items, &Key::Index(index))?;
        let item_id = ops::get_named(realm, &item, "id")?;
        if same_id(&item_id, id)
            && let Value::Object(item) = item
        {
            return Ok(Some(item));
        }
        let submenu = ops::get_named(realm, &item, "submenu")?;
        if matches!(submenu, Value::Object(_)) {
            let children = ops::get_named(realm, &submenu, "items")?;
            if let Some(found) = find_item(realm, &children, id)? {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}

/// Checks the radio item at `index` of `items` and unchecks the other
/// radio items of its group, the radio items around it up to a separator
/// or another kind of item.
fn check_radio(realm: &mut Realm, items: &Value, index: u32) -> Result<(), Thrown> {
    let length = array_length(realm, items)?;
    let is_radio = |realm: &mut Realm, at: u32| -> Result<Option<Value>, Thrown> {
        let item = ops::get(realm, items, &Key::Index(at))?;
        let kind = ops::get_named(realm, &item, "type")?;
        Ok((ops::to_string(realm, &kind)?.to_rust_lossy() == "radio").then_some(item))
    };
    let mut first = index;
    while first > 0 && is_radio(realm, first - 1)?.is_some() {
        first -= 1;
    }
    let mut at = first;
    while at < length {
        let Some(item) = is_radio(realm, at)? else {
            break;
        };
        ops::set(
            realm,
            &item,
            &Key::name("checked"),
            Value::Boolean(at == index),
        )?;
        at += 1;
    }
    Ok(())
}

/// The items array holding the item with a native command, and its index.
fn find_native_command(
    realm: &mut Realm,
    items: &Value,
    command: u16,
) -> Result<Option<(Value, u32)>, Thrown> {
    let length = array_length(realm, items)?;
    for index in 0..length {
        let item = ops::get(realm, items, &Key::Index(index))?;
        if private_command(realm, &item) == Some(command) && matches!(item, Value::Object(_)) {
            return Ok(Some((items.clone(), index)));
        }
        let submenu = ops::get_named(realm, &item, "submenu")?;
        if matches!(submenu, Value::Object(_)) {
            let children = ops::get_named(realm, &submenu, "items")?;
            if let Some(found) = find_native_command(realm, &children, command)? {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}

fn same_id(left: &Value, right: &Value) -> bool {
    ops::strict_equals(left, right)
}

/// Notes that an installed menu may have changed. Pages change menu items
/// in bursts, several properties per keystroke, so the native menus are
/// rebuilt once, by [`flush_menus`] at the end of the host step.
fn menus_changed(realm: &mut Realm) -> Value {
    realm.electron_menus_changed = true;
    Value::Undefined
}

/// Rebuilds the native menus after their items changed, installing only
/// those that differ from what is installed.
pub fn flush_menus(realm: &mut Realm) -> Result<(), Thrown> {
    if !core::mem::take(&mut realm.electron_menus_changed) {
        return Ok(());
    }
    let mut installed = core::mem::take(&mut realm.electron_installed_menus);
    let result = rebuild_menus(realm, &mut installed);
    realm.electron_installed_menus = installed;
    result
}

/// A native menu installation's failure, as the error Electron throws.
fn installed_or_thrown(realm: &mut Realm, result: Result<(), u32>) -> Result<(), Thrown> {
    result.map_err(|code| {
        realm.throw(
            ErrorKind::Error,
            &format!("native menu failed (Win32 error {code})"),
        )
    })
}

fn rebuild_menus(
    realm: &mut Realm,
    installed: &mut Vec<(usize, Vec<app_host::NativeMenuItem>)>,
) -> Result<(), Thrown> {
    if let Some(menu) = realm.electron_application_menu.clone() {
        let items = ops::get_named(realm, &Value::Object(menu), "items")?;
        let native = native_items(realm, &items)?;
        if installed.first() != Some(&(0, native.clone())) {
            // The application menu replaces every window's menu, so window
            // menus are installed again over it.
            installed.clear();
            installed_or_thrown(realm, app_host::set_application_menu(&native))?;
            installed.push((0, native));
        }
    }
    let windows = realm.electron_window_menus.clone();
    for (raw, menu) in windows {
        let items = ops::get_named(realm, &Value::Object(menu), "items")?;
        let native = native_items(realm, &items)?;
        if installed
            .iter()
            .any(|entry| *entry == (raw, native.clone()))
        {
            continue;
        }
        let window = app_host::Window::from_raw(raw);
        installed_or_thrown(realm, app_host::set_window_menu(window, &native))?;
        installed.retain(|(window, _)| *window != raw);
        installed.push((raw, native));
    }
    Ok(())
}

/// The label and accelerator Electron gives an item of a role on Windows
/// when the page gives none.
fn role_defaults(role: &str) -> (&'static str, &'static str) {
    match role.to_ascii_lowercase().as_str() {
        "undo" => ("Undo", "CommandOrControl+Z"),
        "redo" => ("Redo", "Control+Y"),
        "cut" => ("Cut", "CommandOrControl+X"),
        "copy" => ("Copy", "CommandOrControl+C"),
        "paste" => ("Paste", "CommandOrControl+V"),
        "pasteandmatchstyle" => ("Paste and Match Style", "Shift+CommandOrControl+V"),
        "selectall" => ("Select All", "CommandOrControl+A"),
        "delete" => ("Delete", ""),
        "minimize" => ("Minimize", "CommandOrControl+M"),
        "close" => ("Close", "CommandOrControl+W"),
        "quit" => ("Exit", ""),
        "reload" => ("Reload", "CmdOrCtrl+R"),
        "forcereload" => ("Force Reload", "Shift+CmdOrCtrl+R"),
        "toggledevtools" => ("Toggle Developer Tools", "Control+Shift+I"),
        "togglefullscreen" => ("Toggle Full Screen", "F11"),
        "resetzoom" => ("Actual Size", "CommandOrControl+0"),
        "zoomin" => ("Zoom In", "CommandOrControl+Plus"),
        "zoomout" => ("Zoom Out", "CommandOrControl+-"),
        "togglespellchecker" => ("Check Spelling While Typing", ""),
        "about" => ("About", ""),
        "help" => ("Help", ""),
        "window" | "windowmenu" => ("Window", ""),
        "filemenu" => ("File", ""),
        "editmenu" => ("Edit", ""),
        "viewmenu" => ("View", ""),
        _ => ("", ""),
    }
}

/// An accelerator as Chromium shows it in menus on Windows: its modifiers
/// in the order Win, Alt, Ctrl, Shift, then the key, joined by `+`, with
/// the key names of the application locale.
fn shortcut_text(accelerator: &str, locale: &str) -> String {
    let chinese = locale == "zh-CN";
    let mut win = false;
    let mut alt = false;
    let mut ctrl = false;
    let mut shift = false;
    let mut key = String::new();
    // "Ctrl++" ends with a `+` key after its separator.
    let parts: Vec<&str> = match accelerator.strip_suffix("++") {
        Some(rest) => rest.split('+').chain(core::iter::once("+")).collect(),
        None => accelerator.split('+').collect(),
    };
    for (index, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        let last = index + 1 == parts.len();
        match lower.as_str() {
            "command" | "cmd" | "super" | "meta" if !last => win = true,
            "commandorcontrol" | "cmdorctrl" | "control" | "ctrl" if !last => ctrl = true,
            "alt" | "option" | "altgr" if !last => alt = true,
            "shift" if !last => shift = true,
            _ => key = (*part).to_string(),
        }
    }
    let named = |english: &'static str, chinese_name: &'static str| {
        if chinese { chinese_name } else { english }
    };
    let key = match key.to_ascii_lowercase().as_str() {
        "plus" | "+" => "+".to_string(),
        "space" => named("Space", "空格键").to_string(),
        "tab" => named("Tab", "Tab 键").to_string(),
        "enter" | "return" => "Enter".to_string(),
        "backspace" => named("Backspace", "退格").to_string(),
        "delete" | "del" => "Del".to_string(),
        "insert" | "ins" => "Ins".to_string(),
        "escape" | "esc" => "Esc".to_string(),
        "home" => named("Home", "首页").to_string(),
        "end" => named("End", "结束").to_string(),
        "pageup" => named("Page Up", "向上翻页").to_string(),
        "pagedown" => named("Page Down", "向下翻页").to_string(),
        "up" => named("Up Arrow", "向上箭头").to_string(),
        "down" => named("Down Arrow", "向下箭头").to_string(),
        "left" => named("Left Arrow", "向左箭头").to_string(),
        "right" => named("Right Arrow", "向右箭头").to_string(),
        "," => named("Comma", "逗号").to_string(),
        "." => named("Period", "句号").to_string(),
        lower
            if lower.len() > 1
                && lower.starts_with('f')
                && lower[1..].bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            lower.to_ascii_uppercase()
        }
        _ if key.chars().count() == 1 => key.to_uppercase(),
        _ => key,
    };
    let mut text = String::new();
    for (down, name) in [(win, "Win"), (alt, "Alt"), (ctrl, "Ctrl"), (shift, "Shift")] {
        if down {
            text.push_str(name);
            text.push('+');
        }
    }
    text.push_str(&key);
    text
}

/// Drops a trailing `(&X)` mnemonic from labels, as Chromium's popup menus
/// show "主题(&T)": the parenthesized key that CJK labels carry for the
/// Windows menu bar.
fn without_bracketed_mnemonics(items: &mut [app_host::NativeMenuItem]) {
    for item in items {
        let label = &mut item.label;
        // Labels end with their terminating zero.
        let end = label.len().saturating_sub(1);
        if end >= 4
            && label[end - 4] == u16::from(b'(')
            && label[end - 3] == u16::from(b'&')
            && char::from_u32(u32::from(label[end - 2]))
                .is_some_and(|key| key.is_ascii_alphanumeric())
            && label[end - 1] == u16::from(b')')
        {
            label.drain(end - 4..end);
        }
        without_bracketed_mnemonics(&mut item.children);
    }
}

#[cfg(test)]
#[path = "electron_menu/tests.rs"]
mod tests;
