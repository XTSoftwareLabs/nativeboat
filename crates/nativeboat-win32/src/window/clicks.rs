//! Click counting as Chromium counts on Windows: a press repeats the last
//! one when it is the same button with the same modifiers, within the
//! double-click time and rectangle, whatever message brought it. Windows
//! sends `WM_LBUTTONDBLCLK` only for a second press, so a third press of a
//! triple click arrives as a plain press, which pages see with `detail` 3.

use crate::Modifiers;
use core::cell::Cell;
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK,
};

/// A press: when (in microseconds), where on the screen, which button with
/// which modifiers, and its click count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Press {
    pub time: u64,
    pub point: (i32, i32),
    pub button: i16,
    pub modifiers: (bool, bool, bool, bool),
    pub count: u32,
}

/// The system's double-click time in microseconds and rectangle in pixels.
#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    pub time: u64,
    pub width: i32,
    pub height: i32,
}

/// The click count of a press after the last one.
pub(super) fn repeat(last: Option<&Press>, press: &Press, limits: Limits) -> u32 {
    match last {
        Some(last)
            if last.button == press.button
                && last.modifiers == press.modifiers
                && press.time > last.time
                && press.time - last.time <= limits.time
                && (press.point.0 - last.point.0).abs() <= limits.width / 2
                && (press.point.1 - last.point.1).abs() <= limits.height / 2 =>
        {
            last.count + 1
        }
        _ => 1,
    }
}

local! {
    static LAST: Cell<Option<Press>> = const { Cell::new(None) };
}

fn modifiers(modifiers: &Modifiers) -> (bool, bool, bool, bool) {
    (
        modifiers.shift,
        modifiers.ctrl,
        modifiers.alt,
        modifiers.meta,
    )
}

/// Counts a press at a screen point.
pub(super) fn press(button: i16, point: (i32, i32), held: &Modifiers) -> u32 {
    let mut now = FILETIME::default();
    // SAFETY: these read the clock and the system's mouse settings.
    let limits = unsafe {
        GetSystemTimePreciseAsFileTime(&mut now);
        Limits {
            time: u64::from(GetDoubleClickTime()) * 1000,
            width: GetSystemMetrics(SM_CXDOUBLECLK),
            height: GetSystemMetrics(SM_CYDOUBLECLK),
        }
    };
    let time = (u64::from(now.dwHighDateTime) << 32 | u64::from(now.dwLowDateTime)) / 10;
    let mut press = Press {
        time,
        point,
        button,
        modifiers: modifiers(held),
        count: 1,
    };
    LAST.with(|last| {
        press.count = repeat(last.get().as_ref(), &press, limits);
        last.set(Some(press));
    });
    press.count
}

/// The click count of a release: its press's, when it releases the button
/// pressed last.
pub(super) fn release(button: i16) -> u32 {
    LAST.with(|last| {
        last.get()
            .filter(|last| last.button == button)
            .map_or(1, |last| last.count)
    })
}

/// Forgets the last press, so a test's first press counts one.
#[cfg(test)]
pub(super) fn forget() {
    LAST.with(|last| last.set(None));
}

#[cfg(test)]
#[path = "clicks/tests.rs"]
mod tests;
