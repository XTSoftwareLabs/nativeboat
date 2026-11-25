//! Incoming Explorer drags through OLE, including hover feedback.

use crate::event_loop::dispatch;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::narrow;
use crate::{Event, FileDragKind, Modifiers, WindowId, keys, window};
use core::cell::RefCell;
use core::ffi::c_void;
use core::ptr::{null_mut, read, write};
use core::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT_COPY, MK_ALT, RegisterDragDrop, ReleaseStgMedium,
};
use windows_sys::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_sys::core::{GUID, HRESULT, IID_IUnknown, IUnknown_Vtbl};

const IID_DROP_TARGET: GUID = GUID::from_u128(0x00000122_0000_0000_c000_000000000046);

// COM interfaces begin with these exact ABI vtables. Only IDataObject's
// GetData and QueryGetData slots are called; GetDataHere is reserved here.
#[repr(C)]
struct DataObjectVtbl {
    base: IUnknown_Vtbl,
    get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    get_data_here: usize,
    query_get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> HRESULT,
}

#[repr(C)]
struct DropTargetVtbl {
    base: IUnknown_Vtbl,
    enter: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINTL, *mut u32) -> HRESULT,
    over: unsafe extern "system" fn(*mut c_void, u32, POINTL, *mut u32) -> HRESULT,
    leave: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINTL, *mut u32) -> HRESULT,
}

#[repr(C)]
struct DropTarget {
    vtable: *const DropTargetVtbl,
    references: AtomicU32,
    window: WindowId,
    hwnd: HWND,
    paths: RefCell<Vec<String>>,
    /// The text the drag carries, by web type.
    strings: RefCell<Vec<(String, String)>>,
}

static VTABLE: DropTargetVtbl = DropTargetVtbl {
    base: IUnknown_Vtbl {
        QueryInterface: query,
        AddRef: add_ref,
        Release: release,
    },
    enter,
    over,
    leave,
    drop: dropped,
};

pub(crate) fn register(hwnd: HWND, window: WindowId) -> bool {
    let target = Box::into_raw(Box::new(DropTarget {
        vtable: &VTABLE,
        references: AtomicU32::new(1),
        window,
        hwnd,
        paths: RefCell::new(Vec::new()),
        strings: RefCell::new(Vec::new()),
    }))
    .cast();
    // SAFETY: OLE takes its own reference on success. Release the creator's
    // reference on both success and failure, avoiding an unregistered leak.
    unsafe {
        let registered = RegisterDragDrop(hwnd, target) >= 0;
        release(target);
        registered
    }
}

unsafe extern "system" fn query(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || output.is_null() {
        return E_POINTER;
    }
    // SAFETY: COM callers supply readable IID and writable interface storage.
    unsafe {
        write(output, null_mut());
        let iid = read(iid);
        let matches = |expected: GUID| {
            iid.data1 == expected.data1
                && iid.data2 == expected.data2
                && iid.data3 == expected.data3
                && iid.data4 == expected.data4
        };
        if !matches(IID_IUnknown) && !matches(IID_DROP_TARGET) {
            return E_NOINTERFACE;
        }
        write(output, this);
        add_ref(this);
    }
    S_OK
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    // SAFETY: this is a live DropTarget COM interface.
    unsafe {
        (*this.cast::<DropTarget>())
            .references
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    // SAFETY: this caller owns a reference; only the last release frees it.
    let remaining = unsafe {
        (*this.cast::<DropTarget>())
            .references
            .fetch_sub(1, Ordering::AcqRel)
            - 1
    };
    if remaining == 0 {
        // SAFETY: the unique final reference owns the Box allocation.
        unsafe { core::mem::drop(Box::from_raw(this.cast::<DropTarget>())) };
    }
    remaining
}

// A page's callback may close its window and revoke the OLE registration.
// Keep the COM object alive across each dispatch and hold no RefCell borrow.
struct Callback(*mut c_void);
impl Callback {
    unsafe fn new(this: *mut c_void) -> Self {
        // SAFETY: the COM caller has entered a live interface.
        unsafe { add_ref(this) };
        Self(this)
    }
}
impl Drop for Callback {
    fn drop(&mut self) {
        // SAFETY: balances the reference acquired on callback entry.
        unsafe { release(self.0) };
    }
}

unsafe fn paths(data: *mut c_void) -> Vec<String> {
    if data.is_null() {
        return Vec::new();
    }
    let format = FORMATETC {
        cfFormat: CF_HDROP,
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL as u32,
        ..Default::default()
    };
    let mut medium = STGMEDIUM::default();
    // SAFETY: IDataObject exposes its COM vtable; the format and medium live
    // through these calls. CF_HDROP contains paths, never file contents.
    unsafe {
        let table = *data.cast::<*const DataObjectVtbl>();
        if ((*table).query_get_data)(data, &format) < 0
            || ((*table).get_data)(data, &format, &mut medium) < 0
        {
            return Vec::new();
        }
        let mut paths = Vec::new();
        if medium.tymed == TYMED_HGLOBAL as u32 && !medium.u.hGlobal.is_null() {
            let handle = medium.u.hGlobal as HDROP;
            let count = DragQueryFileW(handle, u32::MAX, null_mut(), 0);
            for index in 0..count {
                let length = DragQueryFileW(handle, index, null_mut(), 0);
                let mut path = vec![0_u16; length as usize + 1];
                if DragQueryFileW(handle, index, path.as_mut_ptr(), length + 1) > 0 {
                    paths.push(narrow(&path));
                }
            }
        }
        ReleaseStgMedium(&mut medium);
        paths
    }
}

/// The bytes of a format the drag offers in global memory.
unsafe fn global_bytes(data: *mut c_void, format: u32) -> Option<Vec<u8>> {
    let format = FORMATETC {
        cfFormat: format as u16,
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL as u32,
        ..Default::default()
    };
    let mut medium = STGMEDIUM::default();
    // SAFETY: IDataObject exposes its COM vtable; the medium is released
    // here and its memory is read only while locked.
    unsafe {
        let table = *data.cast::<*const DataObjectVtbl>();
        if ((*table).query_get_data)(data, &format) < 0
            || ((*table).get_data)(data, &format, &mut medium) < 0
        {
            return None;
        }
        let mut bytes = None;
        if medium.tymed == TYMED_HGLOBAL as u32 && !medium.u.hGlobal.is_null() {
            let size = GlobalSize(medium.u.hGlobal);
            let pointer = GlobalLock(medium.u.hGlobal);
            if !pointer.is_null() {
                bytes = Some(core::slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec());
                GlobalUnlock(medium.u.hGlobal);
            }
        }
        ReleaseStgMedium(&mut medium);
        bytes
    }
}

/// The text a drag carries, as browsers expose it: plain text, a link and
/// HTML, by their web types.
unsafe fn strings(data: *mut c_void) -> Vec<(String, String)> {
    if data.is_null() {
        return Vec::new();
    }
    let name = crate::wide::wide("UniformResourceLocatorW");
    // SAFETY: the name is NUL-terminated and outlives the call.
    let link = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    let formats = [
        (u32::from(CF_UNICODETEXT), "text/plain"),
        (link, "text/uri-list"),
        (crate::clipboard::html_format(), "text/html"),
    ];
    let mut found = Vec::new();
    for (format, kind) in formats {
        // SAFETY: as in `global_bytes`.
        let Some(bytes) = (unsafe { global_bytes(data, format) }) else {
            continue;
        };
        let text = if kind == "text/html" {
            crate::clipboard::decode_html(&bytes)
        } else {
            crate::clipboard::decode_text(&bytes)
        };
        found.push((kind.to_owned(), text));
    }
    found
}

fn modifiers(state: u32) -> Modifiers {
    Modifiers {
        ctrl: state & MK_CONTROL != 0,
        shift: state & MK_SHIFT != 0,
        alt: state & MK_ALT != 0,
        meta: keys::modifiers().meta,
    }
}

fn hover(target: &DropTarget, kind: FileDragKind, state: u32, point: POINTL, allowed: u32) -> u32 {
    let paths = target.paths.borrow().clone();
    let strings = target.strings.borrow().clone();
    if paths.is_empty() && strings.is_empty() {
        return 0;
    }
    let (x, y) = window::root_point(target.window, target.hwnd, point.x, point.y, true);
    let accepted = dispatch(
        target.window,
        Event::FileDrag {
            kind,
            paths,
            strings,
            x,
            y,
            modifiers: modifiers(state),
        },
    );
    if accepted && window::hwnd(target.window).is_some() {
        allowed & DROPEFFECT_COPY
    } else {
        0
    }
}

unsafe extern "system" fn enter(
    this: *mut c_void,
    data: *mut c_void,
    state: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    if effect.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: valid COM callback pointers; the guard survives page dispatch.
    unsafe {
        let _callback = Callback::new(this);
        let target = &*this.cast::<DropTarget>();
        *target.paths.borrow_mut() = paths(data);
        *target.strings.borrow_mut() = strings(data);
        *effect = hover(target, FileDragKind::Enter, state, point, *effect);
    }
    S_OK
}

unsafe extern "system" fn over(
    this: *mut c_void,
    state: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    if effect.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: valid COM callback pointers; no borrow crosses page dispatch.
    unsafe {
        let _callback = Callback::new(this);
        *effect = hover(
            &*this.cast::<DropTarget>(),
            FileDragKind::Over,
            state,
            point,
            *effect,
        );
    }
    S_OK
}

unsafe extern "system" fn leave(this: *mut c_void) -> HRESULT {
    // SAFETY: a live COM callback; retain the object through page dispatch.
    unsafe {
        let _callback = Callback::new(this);
        let target = &*this.cast::<DropTarget>();
        let paths = core::mem::take(&mut *target.paths.borrow_mut());
        let strings = core::mem::take(&mut *target.strings.borrow_mut());
        if !paths.is_empty() || !strings.is_empty() {
            dispatch(
                target.window,
                Event::FileDrag {
                    kind: FileDragKind::Leave,
                    paths,
                    strings,
                    x: 0.0,
                    y: 0.0,
                    modifiers: Modifiers::default(),
                },
            );
        }
    }
    S_OK
}

unsafe extern "system" fn dropped(
    this: *mut c_void,
    data: *mut c_void,
    state: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    if effect.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live COM callback, readable data and writable drop effect.
    unsafe {
        let _callback = Callback::new(this);
        let target = &*this.cast::<DropTarget>();
        *target.paths.borrow_mut() = paths(data);
        *target.strings.borrow_mut() = strings(data);
        let accepted = hover(target, FileDragKind::Over, state, point, *effect);
        *effect = 0;
        let paths = core::mem::take(&mut *target.paths.borrow_mut());
        let strings = core::mem::take(&mut *target.strings.borrow_mut());
        if accepted != 0 {
            let (x, y) = window::root_point(target.window, target.hwnd, point.x, point.y, true);
            if dispatch(
                target.window,
                Event::FilesDropped {
                    paths,
                    strings,
                    x,
                    y,
                },
            ) {
                *effect = accepted;
            }
        } else if !paths.is_empty() || !strings.is_empty() {
            dispatch(
                target.window,
                Event::FileDrag {
                    kind: FileDragKind::Leave,
                    paths,
                    strings,
                    x: 0.0,
                    y: 0.0,
                    modifiers: Modifiers::default(),
                },
            );
        }
    }
    S_OK
}

#[cfg(test)]
mod tests;
