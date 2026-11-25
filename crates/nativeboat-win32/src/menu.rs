//! Menu bars of windows, popup menus and keyboard accelerators. Choosing an
//! item, by mouse or by its accelerator, sends [`crate::Event::Menu`] with
//! the item's id to its window.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::wide;
use crate::window::{hwnd, scale};
use crate::{Error, Modifiers, WindowId};
use core::cell::RefCell;
use core::ptr;
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::Controls::EM_SETSEL;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ACCEL, AppendMenuW, CheckMenuItem, CreateAcceleratorTableW, CreateMenu, CreatePopupMenu,
    DestroyAcceleratorTable, DestroyMenu, DrawMenuBar, EnableMenuItem, FALT, FCONTROL, FSHIFT,
    FVIRTKEY, GA_ROOT, GetAncestor, GetCursorPos, HACCEL, HMENU, MF_BYPOSITION, MF_CHECKED,
    MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, SendMessageW, SetForegroundWindow, SetMenu,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateAcceleratorW,
    WM_CLEAR, WM_COPY, WM_CUT, WM_PASTE, WM_UNDO,
};

/// A key combination that chooses a menu item.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Accelerator {
    /// The virtual key code.
    pub key: u16,
    /// Control.
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt.
    pub alt: bool,
    /// How the menu shows it, such as `Ctrl+S`.
    pub label: String,
}

/// What a menu item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuKind {
    /// A command.
    #[default]
    Normal,
    /// A line between groups.
    Separator,
    /// A command with a check mark.
    Checkbox,
    /// One choice of a group, with a check mark.
    Radio,
    /// A nested menu.
    Submenu,
}

/// A menu entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuItem {
    /// The id that choosing the item reports: 1 to 0xEFFF.
    pub id: u16,
    /// The text; `&` marks the access key.
    pub label: String,
    /// What the item is.
    pub kind: MenuKind,
    /// Whether the item can be chosen.
    pub enabled: bool,
    /// Whether a checkbox or radio item shows its check mark.
    pub checked: bool,
    /// Whether the item is omitted from the displayed native menu.
    pub hidden: bool,
    /// Whether its accelerator is shown but not registered.
    pub ignore_accelerator: bool,
    /// The keys that choose the item.
    pub accelerator: Option<Accelerator>,
    /// The items of a submenu.
    pub items: Vec<MenuItem>,
}

struct Bar {
    hwnd: usize,
    menu: HMENU,
    items: Rc<[MenuItem]>,
    accelerators: HACCEL,
}

local! {
    static BARS: RefCell<HashMap<WindowId, Bar>> = RefCell::new(HashMap::new());
    static IGNORED: RefCell<HashSet<WindowId>> = RefCell::new(HashSet::new());
}

/// Enables or suppresses this window's application-menu accelerators.
/// The setting survives menu replacement; mouse commands and global hotkeys
/// are unaffected.
pub fn set_ignore_menu_shortcuts(window: WindowId, ignore: bool) {
    if hwnd(window).is_none() {
        return;
    }
    IGNORED.with_borrow_mut(|ignored| {
        if ignore {
            ignored.insert(window);
        } else {
            ignored.remove(&window);
        }
    });
}

/// Modifier state for mouse commands from popup and tray menus.
pub fn menu_modifiers() -> Modifiers {
    crate::keys::modifiers()
}

fn collect_accelerators(items: &[MenuItem], accelerators: &mut Vec<ACCEL>) {
    for item in items {
        if item.enabled
            && !item.ignore_accelerator
            && item.kind != MenuKind::Separator
            && let Some(accelerator) = &item.accelerator
        {
            let mut virt = FVIRTKEY;
            if accelerator.ctrl {
                virt |= FCONTROL;
            }
            if accelerator.shift {
                virt |= FSHIFT;
            }
            if accelerator.alt {
                virt |= FALT;
            }
            accelerators.push(ACCEL {
                fVirt: virt,
                key: accelerator.key,
                cmd: item.id,
            });
        }
        collect_accelerators(&item.items, accelerators);
    }
}

fn append(menu: HMENU, items: &[MenuItem]) {
    for item in items.iter().filter(|item| !item.hidden) {
        let mut flags = if item.enabled { 0 } else { MF_GRAYED };
        if item.checked {
            flags |= MF_CHECKED;
        }
        let text = match &item.accelerator {
            Some(accelerator) => format!("{}\t{}", item.label, accelerator.label),
            None => item.label.clone(),
        };
        let text = wide(&text);
        // SAFETY: the menu is live and the text outlives each call.
        unsafe {
            match item.kind {
                MenuKind::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null()),
                MenuKind::Submenu => {
                    let child = CreatePopupMenu();
                    append(child, &item.items);
                    AppendMenuW(menu, MF_POPUP | flags, child as usize, text.as_ptr())
                }
                _ => AppendMenuW(menu, MF_STRING | flags, usize::from(item.id), text.as_ptr()),
            }
        };
    }
}

fn forget(window: WindowId) {
    if let Some(bar) = BARS.with_borrow_mut(|bars| bars.remove(&window))
        && !bar.accelerators.is_null()
    {
        // SAFETY: the table came from CreateAcceleratorTableW and is no
        // longer used.
        unsafe { DestroyAcceleratorTable(bar.accelerators) };
    }
}

fn same_structure(old: &[MenuItem], new: &[MenuItem]) -> bool {
    old.len() == new.len()
        && old.iter().zip(new).all(|(old, new)| {
            old.id == new.id
                && old.label == new.label
                && old.kind == new.kind
                && old.hidden == new.hidden
                && old.ignore_accelerator == new.ignore_accelerator
                && old.accelerator == new.accelerator
                && same_structure(&old.items, &new.items)
        })
}

fn update_states(menu: HMENU, old: &[MenuItem], new: &[MenuItem]) {
    for (position, (old, new)) in old
        .iter()
        .zip(new)
        .filter(|(_, new)| !new.hidden)
        .enumerate()
    {
        // SAFETY: structure and visible positions are unchanged, and the live
        // parent owns every submenu for the duration of this synchronous update.
        unsafe {
            if old.enabled != new.enabled {
                EnableMenuItem(
                    menu,
                    position as u32,
                    MF_BYPOSITION | if new.enabled { 0 } else { MF_GRAYED },
                );
            }
            if old.checked != new.checked {
                CheckMenuItem(
                    menu,
                    position as u32,
                    MF_BYPOSITION | if new.checked { MF_CHECKED } else { 0 },
                );
            }
            if new.kind == MenuKind::Submenu {
                update_states(
                    windows_sys::Win32::UI::WindowsAndMessaging::GetSubMenu(menu, position as i32),
                    &old.items,
                    &new.items,
                );
            }
        }
    }
}

fn accelerator_table(items: &[MenuItem]) -> HACCEL {
    let mut entries = Vec::new();
    collect_accelerators(items, &mut entries);
    if entries.is_empty() {
        return ptr::null_mut();
    }
    // SAFETY: Windows copies these entries during the call.
    unsafe { CreateAcceleratorTableW(entries.as_ptr(), entries.len() as i32) }
}

/// Gives a window a menu bar, or removes it with `None`.
pub fn set_menu(window: WindowId, items: Option<&[MenuItem]>) -> Result<(), Error> {
    let handle = hwnd(window).ok_or_else(|| Error {
        message: format!("no window {window}"),
    })?;
    if let Some(items) = items {
        let previous =
            BARS.with_borrow(|bars| bars.get(&window).map(|bar| (bar.menu, bar.items.clone())));
        // Verify the window still owns the recorded menu before reusing it.
        // SAFETY: the window is live.
        let current = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetMenu(handle) };
        if let Some((menu, old)) = previous
            && current == menu
            && same_structure(&old, items)
        {
            if old.as_ref() == items {
                return Ok(());
            }
            update_states(menu, &old, items);
            let table = accelerator_table(items);
            forget(window);
            BARS.with_borrow_mut(|bars| {
                bars.insert(
                    window,
                    Bar {
                        hwnd: handle as usize,
                        menu,
                        items: items.into(),
                        accelerators: table,
                    },
                )
            });
            // SAFETY: the existing live menu bar needs only a state repaint.
            unsafe { DrawMenuBar(handle) };
            return Ok(());
        }
    }
    forget(window);
    let menu = match items {
        Some(items) => {
            // SAFETY: creates an empty menu bar.
            let menu = unsafe { CreateMenu() };
            append(menu, items);
            menu
        }
        None => ptr::null_mut(),
    };
    // SAFETY: the window is live; it owns the new menu and the old one is
    // destroyed with the result of GetMenu being replaced.
    unsafe {
        let old = windows_sys::Win32::UI::WindowsAndMessaging::GetMenu(handle);
        SetMenu(handle, menu);
        if !old.is_null() {
            DestroyMenu(old);
        }
        DrawMenuBar(handle);
    }
    if let Some(items) = items {
        let table = accelerator_table(items);
        BARS.with_borrow_mut(|bars| {
            bars.insert(
                window,
                Bar {
                    hwnd: handle as usize,
                    menu,
                    items: items.into(),
                    accelerators: table,
                },
            )
        });
    }
    Ok(())
}

/// Forgets the menu of a destroyed window.
pub(crate) fn destroyed(window: WindowId) {
    forget(window);
    IGNORED.with_borrow_mut(|ignored| ignored.remove(&window));
}

/// Turns a key message for a window with accelerators into a menu command;
/// returns whether it did.
pub(crate) fn translate(message: &MSG) -> bool {
    if message.hwnd.is_null() {
        return false;
    }
    // SAFETY: the message's window belongs to this thread.
    let root = unsafe { GetAncestor(message.hwnd, GA_ROOT) };
    let table = BARS.with_borrow(|bars| {
        bars.iter()
            .find(|(window, bar)| {
                bar.hwnd == root as usize
                    && !bar.accelerators.is_null()
                    && !IGNORED.with_borrow(|ignored| ignored.contains(window))
            })
            .map(|(_, bar)| bar.accelerators)
    });
    // SAFETY: the window and the table are live and the message is valid.
    table.is_some_and(|table| unsafe { TranslateAcceleratorW(root, table, message) } != 0)
}

/// Shows a popup menu and returns the id of the chosen item. It opens at a
/// point of the window's client area in device-independent pixels, or at
/// the cursor. `owner` receives the menu's messages; without a window the
/// menu belongs to `owner`, such as the notification area's window.
pub fn popup_menu(
    window: Option<WindowId>,
    owner: HWND,
    items: &[MenuItem],
    position: Option<(f32, f32)>,
) -> Option<u16> {
    let target = window.and_then(hwnd).unwrap_or(owner);
    let mut point = POINT::default();
    match (window, position) {
        (Some(window), Some((x, y))) => {
            let scale = scale(window);
            point.x = (x * scale).round() as i32;
            point.y = (y * scale).round() as i32;
            // SAFETY: converts a point of a live window's client area.
            unsafe { ClientToScreen(target, &mut point) };
        }
        // SAFETY: reads the cursor position.
        _ => unsafe {
            GetCursorPos(&mut point);
        },
    }
    // SAFETY: creates an empty popup menu.
    let menu = unsafe { CreatePopupMenu() };
    append(menu, items);
    // SAFETY: the menu and window are live; a menu of an inactive window
    // only closes on outside clicks when the window is in the foreground.
    let chosen = unsafe {
        SetForegroundWindow(target);
        let chosen = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            point.x,
            point.y,
            target,
            ptr::null(),
        );
        DestroyMenu(menu);
        chosen
    };
    u16::try_from(chosen).ok().filter(|id| *id != 0)
}

/// Edit commands that menus send to the focused control: the message and
/// its parameters.
const EDIT_COMMANDS: [(&str, u32, usize, isize); 6] = [
    ("undo", WM_UNDO, 0, 0),
    ("cut", WM_CUT, 0, 0),
    ("copy", WM_COPY, 0, 0),
    ("paste", WM_PASTE, 0, 0),
    ("delete", WM_CLEAR, 0, 0),
    ("selectAll", EM_SETSEL, 0, -1),
];

/// Sends an edit command (`undo`, `cut`, `copy`, `paste`, `delete` or
/// `selectAll`) to the control with the keyboard focus; returns whether
/// there was one to send it to.
pub fn edit_command(command: &str) -> bool {
    let Some(&(_, message, wparam, lparam)) =
        EDIT_COMMANDS.iter().find(|(name, ..)| *name == command)
    else {
        return false;
    };
    // SAFETY: reads this thread's focused window.
    let focus = unsafe { GetFocus() };
    if focus.is_null() {
        return false;
    }
    // SAFETY: the focused window belongs to this thread.
    unsafe { SendMessageW(focus, message, wparam, lparam) };
    true
}

#[cfg(test)]
pub(crate) mod tests;
