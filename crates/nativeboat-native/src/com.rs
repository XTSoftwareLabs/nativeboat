//! The few COM declarations the runtime's Windows features share: interface
//! pointers called through their vtables, created in the apartment the
//! window loop initialized on its thread.

use core::ffi::c_void;

pub(crate) type Hresult = i32;

#[repr(C)]
pub(crate) struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub(crate) const CLSCTX_INPROC_SERVER: u32 = 1;
pub(crate) const S_OK: Hresult = 0;

/// IUnknown's `Release`, after `QueryInterface` and `AddRef`.
const RELEASE: usize = 2;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    pub(crate) fn CoCreateInstance(
        clsid: *const Guid,
        outer: *mut c_void,
        context: u32,
        iid: *const Guid,
        object: *mut *mut c_void,
    ) -> Hresult;
    pub(crate) fn CoTaskMemFree(memory: *mut c_void);
}

/// A COM interface pointer, released when dropped.
pub(crate) struct Com(pub *mut c_void);

impl Com {
    /// A new in-process object of class `clsid`, by its interface `iid`.
    pub(crate) fn create(clsid: &Guid, iid: &Guid) -> Option<Com> {
        let mut object = core::ptr::null_mut();
        // SAFETY: the GUIDs are valid and the out pointer writable.
        let result = unsafe {
            CoCreateInstance(
                clsid,
                core::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                iid,
                &mut object,
            )
        };
        (result == S_OK && !object.is_null()).then(|| Com(object))
    }

    /// The method in a vtable slot.
    ///
    /// # Safety
    /// `F` must be the method's signature.
    pub(crate) unsafe fn method<F: Copy>(&self, slot: usize) -> F {
        // SAFETY: a live interface starts with its vtable pointer, and the
        // caller names the slot's signature.
        unsafe {
            let vtable = *self.0.cast::<*const usize>();
            core::mem::transmute_copy(&*vtable.add(slot))
        }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: the pointer holds one reference, released once.
        unsafe { self.method::<unsafe extern "system" fn(*mut c_void) -> u32>(RELEASE)(self.0) };
    }
}
