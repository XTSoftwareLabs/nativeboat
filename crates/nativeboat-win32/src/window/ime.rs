//! IMM Unicode preedit and candidate geometry for painted editing hosts.
use super::{STATE, px};
use crate::event_loop::dispatch;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{CompositionKind, Event, WindowId};
use core::ffi::c_void;
use core::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::MapWindowPoints;
use windows_sys::Win32::UI::Input::Ime::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Default)]
pub(super) struct State {
    active: bool,
    text: String,
    cursor: usize,
    resolving: bool,
}

struct Context(HWND, HIMC);
impl Context {
    fn of(hwnd: HWND) -> Option<Self> {
        // SAFETY: hwnd belongs to the window processing this message.
        let context = unsafe { ImmGetContext(hwnd) };
        if context.is_null() {
            None
        } else {
            Some(Self(hwnd, context))
        }
    }
    fn text(&self, kind: u32) -> Option<String> {
        // SAFETY: the borrowed input context remains live through both queries.
        let bytes = unsafe { ImmGetCompositionStringW(self.1, kind, null_mut(), 0) };
        if bytes < 0 || bytes % 2 != 0 {
            return None;
        }
        let mut units = vec![0_u16; bytes as usize / 2];
        if bytes == 0 {
            return Some(String::new());
        }
        // SAFETY: units is aligned UTF-16 storage of the queried byte capacity.
        let copied = unsafe {
            ImmGetCompositionStringW(
                self.1,
                kind,
                units.as_mut_ptr().cast::<c_void>(),
                bytes as u32,
            )
        };
        if copied < 0 || copied % 2 != 0 {
            return None;
        }
        units.truncate((copied as usize / 2).min(units.len()));
        Some(String::from_utf16_lossy(&units))
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: balances this owner's ImmGetContext call for the same HWND.
        unsafe { ImmReleaseContext(self.0, self.1) };
    }
}

pub(super) fn active(window: WindowId) -> bool {
    STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .is_some_and(|data| data.ime.active)
    })
}

fn emit(window: WindowId, kind: CompositionKind, data: String, cursor: usize) -> bool {
    let resolving = STATE.with_borrow_mut(|state| {
        if let Some(window) = state.windows.get_mut(&window) {
            window.ime.active = kind != CompositionKind::End;
            window.ime.text = data.clone();
            window.ime.cursor = cursor;
            return window.ime.resolving;
        }
        false
    });
    !resolving && dispatch(window, Event::Composition { kind, data, cursor })
}

/// Returns IMM's final candidate synchronously to a DOM focus/selection change.
/// Suppressing dispatch avoids queuing a result behind the JavaScript operation
/// that is already moving focus; the caller delivers it to the old host.
pub(super) fn resolve(window: WindowId) -> Option<String> {
    let hwnd = STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?;
        if !data.ime.active {
            return None;
        }
        data.ime.resolving = true;
        Some(data.hwnd)
    })?;
    complete(window, hwnd);
    STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?;
        data.ime.resolving = false;
        Some(data.ime.text.clone())
    })
}

fn start(window: WindowId, context: &Context) {
    if active(window) {
        return;
    }
    if emit(window, CompositionKind::Start, String::new(), 0) {
        // End before asking IMM to cancel: its synchronous notifications can
        // reenter the window procedure and must not end this session twice.
        emit(window, CompositionKind::End, String::new(), 0);
        // SAFETY: cancels the live context's composition after DOM cancellation.
        unsafe { ImmNotifyIME(context.1, NI_COMPOSITIONSTR, CPS_CANCEL, 0) };
    }
}

/// Finishes the old painted host before a native or DOM focus change.
pub(super) fn complete(window: WindowId, hwnd: HWND) {
    if !active(window) {
        return;
    }
    if let Some(context) = Context::of(hwnd) {
        // SAFETY: IMM can synchronously deliver result/end messages here.
        unsafe { ImmNotifyIME(context.1, NI_COMPOSITIONSTR, CPS_COMPLETE, 0) };
    }
    if active(window) {
        let text = STATE.with_borrow(|state| state.windows[&window].ime.text.clone());
        let cursor = text.encode_utf16().count();
        emit(window, CompositionKind::End, text, cursor);
    }
}

/// Places native candidate UI beside the rendered caret, in root client pixels.
pub(super) fn position(window: WindowId) {
    if !active(window) {
        return;
    }
    let geometry = STATE.with_borrow(|state| {
        let data = state.windows.get(&window)?;
        let surface = data
            .surfaces
            .values()
            .find(|surface| surface.caret.flatten().is_some())?;
        let rect = surface.caret.flatten()?;
        Some((data.hwnd, surface.hwnd, data.scale, rect, surface.scroll))
    });
    let Some((root, surface, scale, rect, (x, y))) = geometry else {
        return;
    };
    let Some(context) = Context::of(root) else {
        return;
    };
    let mut point = POINT {
        x: px(rect.x - x, scale),
        y: px(rect.y - y, scale),
    };
    // SAFETY: both handles belong to this window; point is writable storage.
    unsafe { MapWindowPoints(surface, root, &mut point, 1) };
    let height = px(rect.height, scale).max(1);
    let composition = COMPOSITIONFORM {
        dwStyle: CFS_POINT,
        ptCurrentPos: point,
        ..Default::default()
    };
    let candidate = CANDIDATEFORM {
        dwIndex: 0,
        dwStyle: CFS_EXCLUDE,
        ptCurrentPos: POINT {
            x: point.x,
            y: point.y + height,
        },
        rcArea: RECT {
            left: point.x,
            top: point.y,
            right: point.x + px(rect.width, scale).max(1),
            bottom: point.y + height,
        },
    };
    // SAFETY: both forms use root client coordinates and live through the calls.
    unsafe {
        ImmSetCompositionWindow(context.1, &composition);
        ImmSetCandidateWindow(context.1, &candidate);
    }
}

pub(super) fn handle(
    window: WindowId,
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> Option<isize> {
    match message {
        WM_IME_SETCONTEXT => {
            // SAFETY: retain native candidate/status UI while the DOM paints
            // preedit text, suppressing only the duplicate composition window.
            Some(unsafe {
                DefWindowProcW(
                    hwnd,
                    message,
                    wparam,
                    lparam & !(ISC_SHOWUICOMPOSITIONWINDOW as isize),
                )
            })
        }
        WM_IME_STARTCOMPOSITION => {
            let context = Context::of(hwnd)?;
            start(window, &context);
            position(window);
            Some(0)
        }
        WM_IME_COMPOSITION => {
            let context = Context::of(hwnd)?;
            let flags = lparam as u32;
            if flags == 0 {
                if active(window) {
                    emit(window, CompositionKind::End, String::new(), 0);
                }
                return Some(0);
            }
            if flags & GCS_RESULTSTR != 0 {
                let text = context.text(GCS_RESULTSTR)?;
                start(window, &context);
                if active(window) {
                    let cursor = text.encode_utf16().count();
                    emit(window, CompositionKind::Update, text.clone(), cursor);
                    emit(window, CompositionKind::End, text, cursor);
                }
            }
            if flags & GCS_COMPSTR != 0 {
                let text = context.text(GCS_COMPSTR)?;
                start(window, &context);
                if active(window) {
                    // SAFETY: GCS_CURSORPOS returns a UTF-16 index, not bytes.
                    let cursor = unsafe {
                        ImmGetCompositionStringW(context.1, GCS_CURSORPOS, null_mut(), 0)
                    };
                    let cursor = if cursor < 0 {
                        text.encode_utf16().count()
                    } else {
                        cursor as usize
                    };
                    emit(window, CompositionKind::Update, text, cursor);
                }
            } else if flags & GCS_CURSORPOS != 0 && active(window) {
                // SAFETY: reads cursor metadata from this live input context.
                let cursor =
                    unsafe { ImmGetCompositionStringW(context.1, GCS_CURSORPOS, null_mut(), 0) };
                if cursor >= 0 {
                    let text = STATE.with_borrow(|state| state.windows[&window].ime.text.clone());
                    emit(window, CompositionKind::Update, text, cursor as usize);
                }
            }
            position(window);
            Some(0)
        }
        WM_IME_ENDCOMPOSITION => {
            if active(window) {
                emit(window, CompositionKind::End, String::new(), 0);
            }
            Some(0)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
