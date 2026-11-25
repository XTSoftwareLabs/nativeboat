//! State of the UI thread without `std`'s thread-local storage. Engine-free
//! applications run all windowing on one thread, so a static holds what
//! `thread_local!` holds with `std`.

use core::cell::{Cell, RefCell, UnsafeCell};

/// A lazily initialized static used only by the UI thread.
pub(crate) struct Local<T: 'static> {
    value: UnsafeCell<Option<T>>,
    initial: fn() -> T,
}

// SAFETY: windowing state is created and used only by the UI thread.
unsafe impl<T> Sync for Local<T> {}

impl<T: 'static> Local<T> {
    pub(crate) const fn new(initial: fn() -> T) -> Self {
        Local {
            value: UnsafeCell::new(None),
            initial,
        }
    }

    fn value(&'static self) -> &'static T {
        // SAFETY: only the UI thread reaches this static, and the value is
        // written once, before any reference to it exists.
        unsafe {
            let slot = self.value.get();
            if (*slot).is_none() {
                let value = (self.initial)();
                *slot = Some(value);
            }
            (*slot).as_ref().unwrap_unchecked()
        }
    }

    pub(crate) fn with<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        f(self.value())
    }

    /// `with`; a static, unlike a thread-local, is never destroyed.
    pub(crate) fn try_with<R>(&'static self, f: impl FnOnce(&T) -> R) -> Result<R, ()> {
        Ok(self.with(f))
    }
}

impl<T: 'static> Local<RefCell<T>> {
    pub(crate) fn with_borrow<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.value().borrow())
    }

    pub(crate) fn with_borrow_mut<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self.value().borrow_mut())
    }
}

impl<T: Copy + 'static> Local<Cell<T>> {
    pub(crate) fn get(&'static self) -> T {
        self.value().get()
    }

    pub(crate) fn set(&'static self, value: T) {
        self.value().set(value);
    }

    pub(crate) fn replace(&'static self, value: T) -> T {
        self.value().replace(value)
    }
}
