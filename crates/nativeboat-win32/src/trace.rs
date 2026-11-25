//! Diagnostics written to the standard error stream when an environment
//! variable asks for them.

use crate::wide::wide;
use core::sync::atomic::{AtomicU8, Ordering};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
}

/// Zero until read, then one when painting is not traced and two when it is.
static PAINT: AtomicU8 = AtomicU8::new(0);

/// Whether `NATIVEBOAT_TRACE_PAINT` asks for the areas windows paint.
pub(crate) fn painting() -> bool {
    let mut state = PAINT.load(Ordering::Relaxed);
    if state == 0 {
        let name = wide("NATIVEBOAT_TRACE_PAINT");
        // SAFETY: the name is NUL-terminated; a null buffer asks for the
        // value's size, zero when it is unset.
        let set = unsafe { GetEnvironmentVariableW(name.as_ptr(), core::ptr::null_mut(), 0) } != 0;
        state = if set { 2 } else { 1 };
        PAINT.store(state, Ordering::Relaxed);
    }
    state == 2
}

/// Writes a line to the standard error stream.
pub(crate) fn line(text: &str) {
    let mut bytes = alloc::format!("nativeboat: {text}\n").into_bytes();
    // SAFETY: the buffer outlives the call; a missing stream fails harmlessly.
    unsafe {
        let handle = GetStdHandle(STD_ERROR_HANDLE);
        let mut written = 0;
        WriteFile(
            handle,
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            &mut written,
            core::ptr::null_mut(),
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn painting_is_traced_only_when_asked() {
        // The tests run without NATIVEBOAT_TRACE_PAINT; the answer is kept.
        assert!(!super::painting());
        assert!(!super::painting());
        super::line("trace test");
    }
}
