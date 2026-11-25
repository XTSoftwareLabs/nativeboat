//! Helpers shared by the tests of this crate.

use crate::{Event, Handler, WindowId, WindowOptions, create_window, set_handler};
use std::cell::{Cell, RefCell};
use std::ptr::null_mut;
use std::rc::Rc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;

/// Isolates this test thread's keyboard state from live desktop modifiers.
/// SetKeyboardState affects this thread only; restore its previous table.
pub(crate) struct KeyboardState([u8; 256]);
impl KeyboardState {
    pub fn neutral() -> Self {
        let mut saved = [0; 256];
        let neutral = [0; 256];
        // SAFETY: each API receives its required 256-byte buffer.
        unsafe {
            assert_ne!(
                windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardState(
                    saved.as_mut_ptr()
                ),
                0
            );
            assert_ne!(
                windows_sys::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(neutral.as_ptr()),
                0
            );
        }
        Self(saved)
    }
}
impl Drop for KeyboardState {
    fn drop(&mut self) {
        // SAFETY: restore only this test thread's saved input-state table.
        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(self.0.as_ptr())
        };
    }
}

/// Events received by the test handler and whether it cancels them.
#[derive(Clone, Default)]
pub(crate) struct Events {
    list: Rc<RefCell<Vec<(WindowId, Event)>>>,
    pub cancel: Rc<Cell<bool>>,
}

impl Events {
    /// Removes and returns the received events.
    pub fn take(&self) -> Vec<Event> {
        self.list
            .borrow_mut()
            .drain(..)
            .map(|(_, event)| event)
            .collect()
    }
}

struct Recorder(Events);

impl Handler for Recorder {
    fn event(&mut self, window: WindowId, event: Event) -> bool {
        self.0.list.borrow_mut().push((window, event));
        self.0.cancel.get()
    }

    fn idle(&mut self) -> Option<Duration> {
        None
    }
}

/// Installs a handler that records events.
pub(crate) fn record() -> Events {
    let events = Events::default();
    set_handler(Box::new(Recorder(events.clone())));
    events
}

/// A hidden window with a 300 by 200 client area.
pub(crate) fn window() -> WindowId {
    create_window(&WindowOptions {
        title: "Test".to_owned(),
        width: 300.0,
        height: 200.0,
        use_content_size: true,
        ..WindowOptions::default()
    })
    .unwrap()
}

pub(crate) fn send(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: tests send messages to windows of their own thread.
    unsafe { SendMessageW(hwnd, message, wparam, lparam) }
}

/// Packs two 16-bit values as a message parameter.
pub(crate) fn pack(low: i32, high: i32) -> isize {
    ((high as u16 as u32) << 16 | low as u16 as u32) as i32 as isize
}

/// A 32-bit top-down bitmap selected into a memory DC.
pub(crate) struct Canvas {
    pub dc: HDC,
    bitmap: HBITMAP,
    bits: *mut u32,
    width: i32,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Canvas {
        crate::init().unwrap();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits = null_mut();
        // SAFETY: the DC and section are released in `drop`.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            SelectObject(dc, bitmap);
            PatBlt(dc, 0, 0, width, height, WHITENESS);
            Canvas {
                dc,
                bitmap,
                bits: bits.cast(),
                width,
            }
        }
    }

    /// The pixel at a point as (r, g, b).
    pub fn pixel(&self, x: i32, y: i32) -> (u8, u8, u8) {
        // SAFETY: callers stay inside the bitmap.
        let value = unsafe { *self.bits.add((y * self.width + x) as usize) };
        ((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }
    /// The pixel at a point as it is stored, alpha and all.
    pub fn bits(&self, x: i32, y: i32) -> u32 {
        // SAFETY: callers stay inside the bitmap.
        unsafe { *self.bits.add((y * self.width + x) as usize) }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: created in `new`.
        unsafe {
            DeleteDC(self.dc);
            DeleteObject(self.bitmap);
        }
    }
}
