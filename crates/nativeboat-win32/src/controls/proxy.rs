//! Controls in rotated or skewed boxes, which no native control can show:
//! each is kept out of sight, the page paints its image through the
//! transform, and input where the image shows reaches it.

use super::*;
use crate::PointerKind;
use windows_sys::Win32::Graphics::GdiPlus::{GdipCreateBitmapFromHBITMAP, GpBitmap, GpImage, Ok};
use windows_sys::Win32::System::SystemServices::{MK_LBUTTON, MK_MBUTTON, MK_RBUTTON};

/// Where a proxied control is kept, in device pixels from its surface's
/// origin: far outside any window.
pub(super) const AWAY: i32 = -32000;

local! {
    /// Set while the page forwards input to a proxied control, whose
    /// coordinates are the control's own already.
    static FORWARDING: Cell<bool> = const { Cell::new(false) };
}

/// A proxied control's transform and its window's scale.
fn transform(window: WindowId, id: u64) -> Option<([f32; 6], f32)> {
    STATE.with_borrow(|state| {
        let data = state.windows.get(&window)?;
        let transform = data.controls.get(&id)?.control.state.transform?;
        Some((transform, data.scale))
    })
}

/// Where a control's client area starts in its box, in device pixels:
/// inside its frame, and lower when the control covers just the middle
/// line of a taller box.
fn client_offset(hwnd: HWND, height: i32) -> (i32, i32) {
    let mut corner = POINT::default();
    let mut bounds = RECT::default();
    // SAFETY: reads the geometry of a live control.
    unsafe {
        ClientToScreen(hwnd, &mut corner);
        GetWindowRect(hwnd, &mut bounds);
    }
    let top = ((height - (bounds.bottom - bounds.top)) / 2).max(0);
    (corner.x - bounds.left, top + corner.y - bounds.top)
}

/// The height of a control's box in device pixels.
fn box_height(window: WindowId, id: u64) -> i32 {
    STATE.with_borrow(|state| {
        state.windows.get(&window).map_or(0, |data| {
            data.controls
                .get(&id)
                .map_or(0, |control| px(control.control.rect.height, data.scale))
        })
    })
}

/// A point of the window's client area, in CSS pixels, in a proxied
/// control's own box, in device pixels; `None` when the transform is
/// singular.
fn local(transform: [f32; 6], scale: f32, x: f32, y: f32) -> Option<(i32, i32)> {
    let [a, b, c, d, e, f] = transform.map(f64::from);
    let determinant = a * d - b * c;
    (determinant.abs() >= 1e-12 && determinant.is_finite()).then(|| {
        let (x, y) = (f64::from(x) - e, f64::from(y) - f);
        let (x, y) = ((d * x - c * y) / determinant, (a * y - b * x) / determinant);
        (px(x as f32, scale), px(y as f32, scale))
    })
}

fn point(x: i32, y: i32) -> LPARAM {
    ((x as u16 as u32) | ((y as u16 as u32) << 16)) as i32 as LPARAM
}

fn mouse(message: u32) -> bool {
    matches!(
        message,
        WM_MOUSEMOVE
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_LBUTTONDBLCLK
            | WM_RBUTTONDOWN
            | WM_RBUTTONUP
            | WM_RBUTTONDBLCLK
            | WM_MBUTTONDOWN
            | WM_MBUTTONUP
            | WM_MBUTTONDBLCLK
    )
}

fn window_of(hwnd: HWND) -> Option<WindowId> {
    STATE.with_borrow(|state| {
        state
            .handles
            .get(&(hwnd as usize))
            .map(|(window, _)| *window)
    })
}

/// Maps the coordinates of mouse input that reaches a proxied control by
/// itself, as while it captures the mouse, from its out-of-sight client
/// area to its own box where its image shows.
pub(super) fn input(hwnd: HWND, id: u64, message: u32, lparam: LPARAM) -> LPARAM {
    if !mouse(message) || FORWARDING.get() {
        return lparam;
    }
    mapped(hwnd, id, lparam).unwrap_or(lparam)
}

fn mapped(hwnd: HWND, id: u64, lparam: LPARAM) -> Option<LPARAM> {
    let window = window_of(hwnd)?;
    let (transform, scale) = transform(window, id)?;
    let root = crate::window::hwnd(window)?;
    let mut at = POINT {
        x: i32::from(lparam as u16 as i16),
        y: i32::from((lparam >> 16) as u16 as i16),
    };
    // SAFETY: both windows are live windows of this thread.
    unsafe {
        ClientToScreen(hwnd, &mut at);
        ScreenToClient(root, &mut at);
    }
    let (x, y) = local(transform, scale, at.x as f32 / scale, at.y as f32 / scale)?;
    let (left, top) = client_offset(hwnd, box_height(window, id));
    Some(point(x - left, y - top))
}

/// The mouse message and key flags of a pointer event.
fn message(kind: PointerKind, button: i16, buttons: u16, count: u32) -> Option<(u32, WPARAM)> {
    let flags = [(1, MK_LBUTTON), (2, MK_RBUTTON), (4, MK_MBUTTON)]
        .into_iter()
        .filter(|(bit, _)| buttons & bit != 0)
        .map(|(_, flag)| flag as WPARAM)
        .sum();
    let message = match (kind, button) {
        (PointerKind::Move, _) => WM_MOUSEMOVE,
        (PointerKind::Down, 0) if count == 2 => WM_LBUTTONDBLCLK,
        (PointerKind::Down, 0) => WM_LBUTTONDOWN,
        (PointerKind::Down, 1) => WM_MBUTTONDOWN,
        (PointerKind::Down, 2) => WM_RBUTTONDOWN,
        (PointerKind::Up, 0) => WM_LBUTTONUP,
        (PointerKind::Up, 1) => WM_MBUTTONUP,
        (PointerKind::Up, 2) => WM_RBUTTONUP,
        _ => return None,
    };
    Some((message, flags))
}

/// Forwards a pointer event the page received where a proxied control's
/// image shows, at a point of the window's client area in CSS pixels:
/// its kind, button, buttons held and click count. `false` when the
/// control is not proxied or takes no such event.
pub fn forward(
    window: WindowId,
    id: u64,
    (kind, button, buttons, count): (PointerKind, i16, u16, u32),
    (x, y): (f32, f32),
) -> bool {
    let send = || {
        let hwnd = hwnd(window, id)?;
        let (transform, scale) = transform(window, id)?;
        let (message, flags) = message(kind, button, buttons, count)?;
        let (x, y) = local(transform, scale, x, y)?;
        let (left, top) = client_offset(hwnd, box_height(window, id));
        let (x, y) = (x - left, y - top);
        FORWARDING.set(true);
        // SAFETY: the control belongs to this live window.
        unsafe { SendMessageW(hwnd, message, flags, point(x, y)) };
        FORWARDING.set(false);
        Some(())
    };
    send().is_some()
}

/// Whether a message can change how a control looks.
pub(super) fn changes(message: u32) -> bool {
    mouse(message)
        || matches!(
            message,
            WM_CHAR
                | WM_KEYDOWN
                | WM_KEYUP
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_SETTEXT
                | WM_PASTE
                | WM_CUT
                | WM_CLEAR
                | WM_UNDO
                | WM_ENABLE
                | WM_IME_COMPOSITION
                | WM_IME_ENDCOMPOSITION
                | EM_SETSEL
                | EM_REPLACESEL
                | BM_SETCHECK
                | BM_SETSTATE
                | CB_SETCURSEL
                | TBM_SETPOS
        )
}

/// Paints a proxied control's image again after a message that can change
/// how it looks.
pub(super) fn after(hwnd: HWND, id: u64, message: u32) {
    if !changes(message) {
        return;
    }
    let parent = window_of(hwnd).and_then(|window| {
        let surface = STATE.with_borrow(|state| {
            let data = state.windows.get(&window)?.controls.get(&id)?;
            data.control.state.transform.map(|_| data.surface)
        })?;
        surface_hwnd(window, surface)
    });
    if let Some(parent) = parent {
        // SAFETY: invalidates a live surface of this window.
        unsafe { InvalidateRect(parent, null(), 0) };
    }
}

/// Shows the calendar a proxied date picker drops down below where its
/// image shows, rather than below the picker out of sight.
pub(super) fn dropped(window: WindowId, picker: HWND) {
    let place = || {
        let (id, _) = control_of(window, picker)?;
        let (transform, scale) = transform(window, id)?;
        let root = crate::window::hwnd(window)?;
        let mut bounds = RECT::default();
        // SAFETY: reads the size of the live picker.
        unsafe { GetClientRect(picker, &mut bounds) };
        let (width, height) = (bounds.right as f32 / scale, bounds.bottom as f32 / scale);
        let [a, b, c, d, e, f] = transform;
        let corners = [(0.0, 0.0), (width, 0.0), (0.0, height), (width, height)]
            .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
        let left = corners
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::INFINITY, f32::min);
        let bottom = corners
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::NEG_INFINITY, f32::max);
        let mut at = POINT {
            x: px(left, scale),
            y: px(bottom, scale),
        };
        // SAFETY: the calendar's drop-down window belongs to the live
        // picker; it moves without being activated.
        unsafe {
            ClientToScreen(root, &mut at);
            let calendar = SendMessageW(picker, DTM_GETMONTHCAL, 0, 0) as HWND;
            // The calendar sits in a pop-up window of its own.
            let popup = Some(calendar)
                .filter(|calendar| !calendar.is_null())
                .map(|calendar| GetParent(calendar))
                .filter(|popup| !popup.is_null() && *popup != picker)?;
            SetWindowPos(
                popup,
                null_mut(),
                at.x,
                at.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        Some(())
    };
    place();
}

/// Keeps a proxied control out of sight, at its size.
pub(super) fn hide(hwnd: HWND) {
    // SAFETY: the handle belongs to a live control of this thread.
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            AWAY,
            AWAY,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// The image of a proxied control: a GDI+ bitmap of its box, which the
/// caller disposes, its size in pixels, and its id.
pub(crate) struct Snapshot {
    pub id: u64,
    pub image: *mut GpImage,
    pub size: (u32, u32),
}

/// What printing a proxied control takes: its window and id, its size in
/// device pixels, its text color and the height of its caret.
struct Print {
    window: WindowId,
    id: u64,
    hwnd: HWND,
    size: (i32, i32),
    color: Color,
    caret: i32,
}

/// Prints the proxied controls of a surface, each with its caret while it
/// has the keyboard focus, for the renderer to paint at their markers in
/// their boxes.
pub(crate) fn snapshots(window: WindowId, surface: u64) -> Vec<(Snapshot, nativeboat_ir::Rect)> {
    let proxied: Vec<(Print, nativeboat_ir::Rect)> = STATE.with_borrow(|state| {
        let Some(data) = state.windows.get(&window) else {
            return Vec::new();
        };
        let scale = data.scale;
        data.controls
            .iter()
            .filter(|(_, control)| {
                control.surface == surface && control.control.state.transform.is_some()
            })
            .map(|(id, control)| {
                let rect = control.control.rect;
                let state = &control.control.state;
                let print = Print {
                    window,
                    id: *id,
                    hwnd: control.hwnd,
                    size: (px(rect.width, scale), px(rect.height, scale)),
                    color: state.color,
                    caret: px(fonts::cell_height(&state.font, scale), scale),
                };
                (print, rect)
            })
            .collect()
    });
    proxied
        .into_iter()
        .filter_map(|(control, rect)| {
            let size = (control.size.0 as u32, control.size.1 as u32);
            let id = control.id;
            print(&control).map(|image| (Snapshot { id, image, size }, rect))
        })
        .collect()
}

/// Prints a control, frame and all, into a GDI+ bitmap of its size.
fn print(control: &Print) -> Option<*mut GpImage> {
    let (width, height) = control.size;
    if width <= 0 || height <= 0 {
        return None;
    }
    let hwnd = control.hwnd;
    // SAFETY: the control is live; every temporary GDI object is released
    // here, and GDI+ copies the pixels into its own bitmap.
    unsafe {
        let screen = GetDC(null_mut());
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        ReleaseDC(null_mut(), screen);
        if bitmap.is_null() {
            DeleteDC(memory);
            return None;
        }
        let previous = SelectObject(memory, bitmap);
        PatBlt(memory, 0, 0, width, height, WHITENESS);
        // A field shorter than its box, covering just its line, shows in
        // the middle of the box.
        let (left, top) = client_offset(hwnd, height);
        let mut bounds = RECT::default();
        GetWindowRect(hwnd, &mut bounds);
        let mut origin = POINT::default();
        SetViewportOrgEx(
            memory,
            0,
            ((height - (bounds.bottom - bounds.top)) / 2).max(0),
            &mut origin,
        );
        let printing = PRINTING.replace(hwnd as usize);
        SendMessageW(
            hwnd,
            WM_PRINT,
            memory as usize,
            (PRF_NONCLIENT | PRF_CLIENT | PRF_ERASEBKGND) as isize,
        );
        PRINTING.set(printing);
        // The client area starts inside the frame.
        SetViewportOrgEx(memory, left, top, null_mut());
        paint_placeholder(control.window, control.id, hwnd, memory);
        // The caret blinks in the field; its image shows it steadily.
        let mut caret = POINT::default();
        if GetFocus() == hwnd && GetCaretPos(&mut caret) != 0 {
            let brush = CreateSolidBrush(colorref(control.color));
            let area = RECT {
                left: caret.x,
                top: caret.y,
                right: caret.x + px(1.0, crate::window::scale(control.window)).max(1),
                bottom: caret.y + control.caret,
            };
            FillRect(memory, &area, brush);
            DeleteObject(brush);
        }
        SetViewportOrgEx(memory, origin.x, origin.y, null_mut());
        SelectObject(memory, previous);
        DeleteDC(memory);
        let mut image: *mut GpBitmap = null_mut();
        let made = GdipCreateBitmapFromHBITMAP(bitmap, null_mut(), &mut image) == Ok;
        DeleteObject(bitmap);
        made.then_some(image.cast())
    }
}

#[cfg(test)]
mod tests;
