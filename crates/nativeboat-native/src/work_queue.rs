//! Owned native work on the Windows thread pool. Only `Send` data crosses
//! threads; source values and completion callbacks stay on the realm thread.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

type Handle = *mut c_void;
type Callback = unsafe extern "system" fn(Handle, Handle);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateEventW(security: Handle, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn TrySubmitThreadpoolCallback(callback: Callback, context: Handle, environment: Handle)
    -> i32;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForSingleObject(event: Handle, milliseconds: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
}

/// The OS boundary also permits deterministic resource-failure regression tests.
#[derive(Clone, Copy)]
struct Api {
    create: unsafe extern "system" fn(Handle, i32, i32, *const u16) -> Handle,
    submit: unsafe extern "system" fn(Callback, Handle, Handle) -> i32,
    wait: unsafe extern "system" fn(Handle, u32) -> u32,
    close: unsafe extern "system" fn(Handle) -> i32,
    error: unsafe extern "system" fn() -> u32,
}

const API: Api = Api {
    create: CreateEventW,
    submit: TrySubmitThreadpoolCallback,
    wait: WaitForSingleObject,
    close: CloseHandle,
    error: GetLastError,
};

/// One auto-reset event wakes the owning event loop for any completed request.
pub(crate) struct Signal {
    handle: Handle,
    api: Api,
}

// SAFETY: Windows event handles can be signaled/waited across threads. Arc
// ownership keeps the handle open until the last worker and realm release it.
unsafe impl Send for Signal {}
// SAFETY: shared access only signals/waits on the thread-safe Windows event.
unsafe impl Sync for Signal {}

impl Signal {
    pub(crate) fn new() -> Result<Arc<Self>, u32> {
        Self::create(API)
    }

    fn create(api: Api) -> Result<Arc<Self>, u32> {
        // SAFETY: an unnamed, non-inheritable, initially clear auto-reset event.
        let handle = unsafe { (api.create)(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        if handle.is_null() {
            // SAFETY: retrieves this thread's error before any cleanup changes it.
            return Err(unsafe { (api.error)() });
        }
        Ok(Arc::new(Self { handle, api }))
    }

    /// The Windows event handle, for waits that combine several objects.
    pub(crate) fn raw(&self) -> usize {
        self.handle as usize
    }

    pub(crate) fn wait(&self, milliseconds: u32) -> Result<bool, u32> {
        // SAFETY: self owns the valid event for the duration of the wait.
        let status = unsafe { (self.api.wait)(self.handle, milliseconds) };
        match status {
            0 => Ok(true),
            258 => Ok(false), // WAIT_TIMEOUT.
            // SAFETY: retrieves the calling thread's failure immediately.
            _ => Err(unsafe { (self.api.error)() }),
        }
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        // SAFETY: this is the sole handle owner; the final Arc has been released.
        unsafe { (self.api.close)(self.handle) };
    }
}

struct Shared<T> {
    signal: Arc<Signal>,
    ready: AtomicBool,
    result: UnsafeCell<Option<T>>,
}

// SAFETY: the worker writes once before publishing ready with Release. Only
// the owning Work reads/takes the result after an Acquire load; T is Send.
unsafe impl<T: Send> Sync for Shared<T> {}

struct Payload<T> {
    shared: Arc<Shared<T>>,
    work: Box<dyn FnOnce() -> T + Send>,
}

unsafe extern "system" fn complete<T: Send>(_: Handle, context: Handle) {
    // SAFETY: a successful submission transfers this Box to exactly one callback.
    let Payload { shared, work } = *unsafe { Box::from_raw(context.cast::<Payload<T>>()) };
    let result = work();
    // SAFETY: this callback is the only writer; no reader accesses it until ready.
    unsafe { *shared.result.get() = Some(result) };
    shared.ready.store(true, Ordering::Release);
    // SAFETY: the worker's shared Arc keeps the event alive through this call.
    unsafe { SetEvent(shared.signal.handle) };
}

/// A result polled only by the thread that submitted it. Dropping it before
/// completion releases the result safely after the worker finishes.
pub(crate) struct Work<T> {
    shared: Arc<Shared<T>>,
}

impl<T: Send + 'static> Work<T> {
    pub(crate) fn submit(
        signal: Arc<Signal>,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<Self, u32> {
        let shared = Arc::new(Shared {
            signal,
            ready: AtomicBool::new(false),
            result: UnsafeCell::new(None),
        });
        let payload = Box::into_raw(Box::new(Payload {
            shared: shared.clone(),
            work: Box::new(work),
        }));
        let submit = shared.signal.api.submit;
        // SAFETY: payload remains owned by this call on failure and transfers
        // to the callback on success. The default Windows pool owns scheduling.
        let submitted = unsafe { submit(complete::<T>, payload.cast(), core::ptr::null_mut()) };
        if submitted == 0 {
            // SAFETY: preserve the failure before dropping user captures.
            let error = unsafe { (shared.signal.api.error)() };
            // SAFETY: failure did not schedule a callback, so we still own payload.
            unsafe { drop(Box::from_raw(payload)) };
            return Err(error);
        }
        Ok(Self { shared })
    }

    pub(crate) fn take(&mut self) -> Option<T> {
        if !self.shared.ready.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: the worker has published its final write. Work is not Clone,
        // and this mutable access is the sole reader/taker of the result.
        unsafe { (*self.shared.result.get()).take() }
    }
}

#[cfg(test)]
pub(crate) mod tests;
