//! Icons in the notification area and their balloon notifications. A
//! message-only window receives their mouse input, which reaches the
//! handler as [`crate::Event::Tray`] for window `0`.

use crate::dialogs::MessageKind;
use crate::event_loop::dispatch;
use crate::menu::{MenuItem, popup_menu};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::render::icon;
use crate::wide::wide;
use crate::window::instance;
use crate::{Error, Event};
use core::cell::RefCell;
use core::ptr;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_NONE,
    NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIN_BALLOONUSERCLICK, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, HICON, HWND_MESSAGE, IDI_APPLICATION, LoadIconW,
    RegisterClassW, WM_APP, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};

/// Mouse input on a notification area icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// A left click.
    Click,
    /// A right click.
    RightClick,
    /// A double click.
    DoubleClick,
    /// A click on the icon's balloon notification.
    BalloonClick,
}

const TRAY_MESSAGE: u32 = WM_APP + 2;

local! {
    static WINDOW: RefCell<usize> = const { RefCell::new(0) };
    /// Icons created from images, by tray id, to destroy when replaced.
    static ICONS: RefCell<HashMap<u32, usize>> = RefCell::new(HashMap::new());
}

unsafe extern "system" fn tray_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == TRAY_MESSAGE {
        let event = match lparam as u32 {
            WM_LBUTTONUP => Some(TrayEvent::Click),
            WM_RBUTTONUP => Some(TrayEvent::RightClick),
            WM_LBUTTONDBLCLK => Some(TrayEvent::DoubleClick),
            NIN_BALLOONUSERCLICK => Some(TrayEvent::BalloonClick),
            _ => None,
        };
        if let Some(event) = event {
            dispatch(
                0,
                Event::Tray {
                    id: wparam as u32,
                    event,
                },
            );
        }
        return 0;
    }
    // SAFETY: forwards the message unchanged.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// The window that receives notification area messages, created once.
pub(crate) fn owner() -> Result<HWND, Error> {
    let existing = WINDOW.with_borrow(|window| *window);
    if existing != 0 {
        return Ok(existing as HWND);
    }
    let class = wide("nativeboat-tray");
    let description = WNDCLASSW {
        lpfnWndProc: Some(tray_proc),
        hInstance: instance(),
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    // SAFETY: the class name outlives both calls; a message-only window has
    // no size or style.
    let window = unsafe {
        RegisterClassW(&description);
        CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            instance(),
            ptr::null(),
        )
    };
    if window.is_null() {
        return Err(Error::last("CreateWindowExW"));
    }
    WINDOW.with_borrow_mut(|slot| *slot = window as usize);
    Ok(window)
}

fn copy(target: &mut [u16], text: &str) {
    let units: Vec<u16> = text.encode_utf16().take(target.len() - 1).collect();
    target[..units.len()].copy_from_slice(&units);
    target[units.len()] = 0;
}

/// The icon of an image added with [`crate::add_image`], or the default
/// application icon.
fn tray_icon(id: u32, image: Option<&str>) -> HICON {
    let created = image.and_then(icon);
    let old = ICONS.with_borrow_mut(|icons| match created {
        Some(handle) => icons.insert(id, handle as usize),
        None => icons.remove(&id),
    });
    if let Some(old) = old {
        // SAFETY: the replaced icon was created for this tray icon only.
        unsafe { DestroyIcon(old as HICON) };
    }
    // SAFETY: loads a stock system icon.
    created.unwrap_or_else(|| unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) })
}

fn data(id: u32) -> Result<NOTIFYICONDATAW, Error> {
    Ok(NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: owner()?,
        uID: id,
        ..Default::default()
    })
}

fn notify(message: u32, data: &NOTIFYICONDATAW) -> Result<(), Error> {
    // SAFETY: the structure is complete for the flags it sets.
    if unsafe { Shell_NotifyIconW(message, data) } == 0 {
        return Err(Error::last("Shell_NotifyIconW"));
    }
    Ok(())
}

/// Adds an icon to the notification area.
pub fn add_tray(id: u32, image: Option<&str>, tooltip: &str) -> Result<(), Error> {
    let mut data = data(id)?;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = tray_icon(id, image);
    copy(&mut data.szTip, tooltip);
    notify(NIM_ADD, &data)
}

/// Changes the image of a notification area icon.
pub fn set_tray_image(id: u32, image: Option<&str>) -> Result<(), Error> {
    let mut data = data(id)?;
    data.uFlags = NIF_ICON;
    data.hIcon = tray_icon(id, image);
    notify(NIM_MODIFY, &data)
}

/// Changes the tooltip of a notification area icon.
pub fn set_tray_tooltip(id: u32, tooltip: &str) -> Result<(), Error> {
    let mut data = data(id)?;
    data.uFlags = NIF_TIP | NIF_SHOWTIP;
    copy(&mut data.szTip, tooltip);
    notify(NIM_MODIFY, &data)
}

/// Removes an icon from the notification area.
pub fn remove_tray(id: u32) -> Result<(), Error> {
    let data = data(id)?;
    tray_icon(id, None);
    notify(NIM_DELETE, &data)
}

/// Shows a balloon notification from a notification area icon; Windows 10
/// and later show it as a toast.
pub fn tray_balloon(id: u32, title: &str, body: &str, kind: MessageKind) -> Result<(), Error> {
    let mut data = data(id)?;
    data.uFlags = NIF_INFO;
    copy(&mut data.szInfoTitle, title);
    copy(&mut data.szInfo, body);
    data.dwInfoFlags = match kind {
        MessageKind::Warning => NIIF_WARNING,
        MessageKind::Error => NIIF_ERROR,
        MessageKind::Info | MessageKind::Question => NIIF_INFO,
        MessageKind::None => NIIF_NONE,
    };
    notify(NIM_MODIFY, &data)
}

/// Shows a popup menu for the notification area at the cursor and returns
/// the chosen item's id.
pub fn tray_menu(items: &[MenuItem]) -> Result<Option<u16>, Error> {
    Ok(popup_menu(None, owner()?, items, None))
}

#[cfg(test)]
mod tests;
