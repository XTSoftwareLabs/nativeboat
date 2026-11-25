//! The message loop and event delivery.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Event, Handler, WindowId};
use core::cell::RefCell;
use core::ptr::null_mut;
use core::time::Duration;
use windows_sys::Win32::System::Threading::{GetCurrentThreadId, INFINITE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

local! {
    static HANDLER: RefCell<Option<Box<dyn Handler>>> = RefCell::new(None);
    static QUEUE: RefCell<VecDeque<(WindowId, Event)>> = const { RefCell::new(VecDeque::new()) };
}

/// Installs the handler that receives events on this thread and returns the
/// previous one. Must not be called from inside the handler.
pub fn set_handler(handler: Box<dyn Handler>) -> Option<Box<dyn Handler>> {
    HANDLER.with_borrow_mut(|slot| slot.replace(handler))
}

/// Removes the handler of this thread. Must not be called from inside the
/// handler.
pub fn take_handler() -> Option<Box<dyn Handler>> {
    HANDLER.with_borrow_mut(Option::take)
}

fn drain(handler: &mut dyn Handler) {
    while let Some((window, event)) = QUEUE.with_borrow_mut(VecDeque::pop_front) {
        if current(window, &event) {
            handler.event(window, event);
        }
    }
}

/// Reentrant native focus notifications may be superseded before delivery.
/// Synchronous WM_KILLFOCUS still runs during the native transition itself.
fn current(window: WindowId, event: &Event) -> bool {
    use crate::ControlEvent;
    let Event::Control {
        id,
        event: focus @ (ControlEvent::Focus | ControlEvent::Blur),
    } = event
    else {
        return true;
    };
    let Some(handle) = crate::controls::hwnd(window, *id) else {
        return false;
    };
    // SAFETY: inspect this UI thread's focus after the deferred notification's
    // window procedure has returned.
    let focused = unsafe { GetFocus() } == handle;
    focused == matches!(focus, ControlEvent::Focus)
}

/// Delivers an event to the handler and returns whether it was cancelled.
///
/// Events raised while the handler runs (for example by changes it applies)
/// are queued and delivered when it returns; they cannot be cancelled.
/// Superseded deferred native focus notifications are skipped.
pub fn dispatch(window: WindowId, event: Event) -> bool {
    HANDLER.with(|slot| match slot.try_borrow_mut() {
        Ok(mut slot) => {
            let Some(handler) = slot.as_mut() else {
                return false;
            };
            let cancelled = handler.event(window, event);
            drain(handler.as_mut());
            cancelled
        }
        Err(_) => {
            QUEUE.with_borrow_mut(|queue| queue.push_back((window, event)));
            false
        }
    })
}

/// Hit queries cannot be queued: Windows needs the result before routing input.
pub(crate) fn above_control(window: WindowId, control: u64, x: f32, y: f32) -> bool {
    HANDLER.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return false;
        };
        slot.as_mut()
            .is_some_and(|handler| handler.above_control(window, control, x, y))
    })
}

fn idle() -> Option<Duration> {
    HANDLER.with_borrow_mut(|slot| {
        let handler = slot.as_mut()?;
        let wait = handler.idle();
        drain(handler.as_mut());
        wait
    })
}

/// Runs the message loop with a handler until [`quit`] is called, then
/// returns the handler.
pub fn run(handler: Box<dyn Handler>) -> Box<dyn Handler> {
    set_handler(handler);
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is writable; messages are dispatched to their
        // window procedures on this thread.
        while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if message.message == WM_QUIT {
                return take_handler().expect("the handler is installed while running");
            }
            if message.message == WM_HOTKEY && message.hwnd.is_null() {
                dispatch(0, Event::HotKey(message.wParam as u32));
                continue;
            }
            if crate::window::before_input(&message) || crate::menu::translate(&message) {
                continue;
            }
            // SAFETY: the message was just retrieved.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let timeout = idle().map_or(INFINITE, |wait| {
            wait.as_micros()
                .div_ceil(1000)
                .min(u128::from(INFINITE - 1)) as u32
        });
        // SAFETY: no handles are passed.
        unsafe {
            MsgWaitForMultipleObjectsEx(
                0,
                core::ptr::null(),
                timeout,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
    }
}

/// Registers a key combination for the whole system; pressing it sends
/// [`Event::HotKey`] with `id` to window `0`. Returns whether the
/// combination was free.
pub fn register_hotkey(id: u32, accelerator: &crate::Accelerator) -> bool {
    let mut modifiers = MOD_NOREPEAT;
    if accelerator.ctrl {
        modifiers |= MOD_CONTROL;
    }
    if accelerator.shift {
        modifiers |= MOD_SHIFT;
    }
    if accelerator.alt {
        modifiers |= MOD_ALT;
    }
    // SAFETY: registers for this thread's message queue.
    unsafe { RegisterHotKey(null_mut(), id as i32, modifiers, u32::from(accelerator.key)) != 0 }
}

/// Releases a key combination registered with [`register_hotkey`].
pub fn unregister_hotkey(id: u32) {
    // SAFETY: unregisters this thread's hotkey; unknown ids are ignored.
    unsafe { UnregisterHotKey(null_mut(), id as i32) };
}

/// Ends the message loop of this thread.
pub fn quit() {
    // SAFETY: posts WM_QUIT to this thread's queue.
    unsafe { PostQuitMessage(0) };
}

/// Wakes the message loop of a thread from any thread, so that its handler's
/// `idle` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waker {
    thread: u32,
}

impl Waker {
    /// A waker for the calling thread's loop.
    pub fn current() -> Waker {
        Waker {
            // SAFETY: reads the calling thread's id.
            thread: unsafe { GetCurrentThreadId() },
        }
    }

    /// Wakes the loop.
    pub fn wake(&self) {
        // SAFETY: posting a message to a thread has no memory effects here.
        unsafe { PostThreadMessageW(self.thread, WM_APP, 0, 0) };
    }
}

#[cfg(test)]
mod tests;
