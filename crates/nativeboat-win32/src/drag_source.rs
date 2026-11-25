//! Outgoing drags to other applications through OLE: the text, links and
//! HTML a page drags, offered to whatever window it is dropped on.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::wide;
use core::ffi::c_void;
use core::ptr::{null_mut, read, write};
use core::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::{
    CF_UNICODETEXT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DoDragDrop,
};
use windows_sys::Win32::System::SystemServices::MK_LBUTTON;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::Shell::SHCreateStdEnumFmtEtc;
use windows_sys::core::{GUID, HRESULT, IID_IUnknown};

const IID_DATA_OBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
const IID_DROP_SOURCE: GUID = GUID::from_u128(0x00000121_0000_0000_c000_000000000046);
const DRAGDROP_S_DROP: HRESULT = 0x0004_0100;
const DRAGDROP_S_CANCEL: HRESULT = 0x0004_0101;
const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = 0x0004_0102;
const DATA_S_SAMEFORMATETC: HRESULT = 0x0004_0130;
const OLE_E_ADVISENOTSUPPORTED: HRESULT = 0x8004_0003_u32 as HRESULT;

#[repr(C)]
struct DataObjectVtbl {
    query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    get_data_here:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    query_get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> HRESULT,
    canonical: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut FORMATETC) -> HRESULT,
    set_data:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *const STGMEDIUM, i32) -> HRESULT,
    enumerate: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    advise: unsafe extern "system" fn(
        *mut c_void,
        *const FORMATETC,
        u32,
        *mut c_void,
        *mut u32,
    ) -> HRESULT,
    unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    advisers: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DropSourceVtbl {
    query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    keep_going: unsafe extern "system" fn(*mut c_void, i32, u32) -> HRESULT,
    feedback: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

/// A COM object: its interface table, references and contents.
#[repr(C)]
struct Object<V: 'static, T> {
    vtable: &'static V,
    references: AtomicU32,
    /// The interface it answers to besides `IUnknown`.
    iid: GUID,
    contents: T,
}

/// The formats a drag offers, by clipboard format, as global memory bytes.
type Formats = Vec<(u16, Vec<u8>)>;

static DATA_VTABLE: DataObjectVtbl = DataObjectVtbl {
    query: query::<DataObjectVtbl, Formats>,
    add_ref: add_ref::<DataObjectVtbl, Formats>,
    release: release::<DataObjectVtbl, Formats>,
    get_data,
    get_data_here,
    query_get_data,
    canonical,
    set_data,
    enumerate,
    advise,
    unadvise,
    advisers,
};

static SOURCE_VTABLE: DropSourceVtbl = DropSourceVtbl {
    query: query::<DropSourceVtbl, ()>,
    add_ref: add_ref::<DropSourceVtbl, ()>,
    release: release::<DropSourceVtbl, ()>,
    keep_going,
    feedback,
};

fn same(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

unsafe extern "system" fn query<V: 'static, T>(
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
        let object = &*this.cast::<Object<V, T>>();
        if !same(&iid, &IID_IUnknown) && !same(&iid, &object.iid) {
            return E_NOINTERFACE;
        }
        object.references.fetch_add(1, Ordering::Relaxed);
        write(output, this);
    }
    S_OK
}

unsafe extern "system" fn add_ref<V: 'static, T>(this: *mut c_void) -> u32 {
    // SAFETY: this is a live object of this type.
    unsafe {
        (*this.cast::<Object<V, T>>())
            .references
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
}

unsafe extern "system" fn release<V: 'static, T>(this: *mut c_void) -> u32 {
    // SAFETY: the caller owns a reference; only the last release frees it.
    let remaining = unsafe {
        (*this.cast::<Object<V, T>>())
            .references
            .fetch_sub(1, Ordering::AcqRel)
            - 1
    };
    if remaining == 0 {
        // SAFETY: the final reference owns the Box allocation.
        unsafe { core::mem::drop(Box::from_raw(this.cast::<Object<V, T>>())) };
    }
    remaining
}

/// The offered bytes of a format the caller asks for in global memory.
fn offered<'a>(this: *mut c_void, format: *const FORMATETC) -> Option<&'a [u8]> {
    if format.is_null() {
        return None;
    }
    // SAFETY: a live data object and a readable format from the caller.
    unsafe {
        let format = &*format;
        if format.tymed & TYMED_HGLOBAL as u32 == 0 || format.dwAspect != DVASPECT_CONTENT {
            return None;
        }
        let formats = &(*this.cast::<Object<DataObjectVtbl, Formats>>()).contents;
        formats
            .iter()
            .find(|(kind, _)| *kind == format.cfFormat)
            .map(|(_, bytes)| bytes.as_slice())
    }
}

unsafe extern "system" fn get_data(
    this: *mut c_void,
    format: *const FORMATETC,
    medium: *mut STGMEDIUM,
) -> HRESULT {
    if medium.is_null() {
        return E_POINTER;
    }
    let Some(bytes) = offered(this, format) else {
        return DV_E_FORMATETC;
    };
    // SAFETY: the new global block is filled while locked and handed to the
    // caller, who releases it; the medium is writable.
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if handle.is_null() {
            return E_OUTOFMEMORY;
        }
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            return E_OUTOFMEMORY;
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        GlobalUnlock(handle);
        write(
            medium,
            STGMEDIUM {
                tymed: TYMED_HGLOBAL as u32,
                u: windows_sys::Win32::System::Com::STGMEDIUM_0 { hGlobal: handle },
                pUnkForRelease: null_mut(),
            },
        );
    }
    S_OK
}

unsafe extern "system" fn get_data_here(
    _: *mut c_void,
    _: *const FORMATETC,
    _: *mut STGMEDIUM,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn query_get_data(this: *mut c_void, format: *const FORMATETC) -> HRESULT {
    if offered(this, format).is_some() {
        S_OK
    } else {
        DV_E_FORMATETC
    }
}

unsafe extern "system" fn canonical(
    _: *mut c_void,
    _: *const FORMATETC,
    output: *mut FORMATETC,
) -> HRESULT {
    if !output.is_null() {
        // SAFETY: the caller's output is writable.
        unsafe { (*output).ptd = null_mut() };
    }
    DATA_S_SAMEFORMATETC
}

unsafe extern "system" fn set_data(
    _: *mut c_void,
    _: *const FORMATETC,
    _: *const STGMEDIUM,
    _: i32,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn enumerate(
    this: *mut c_void,
    direction: u32,
    output: *mut *mut c_void,
) -> HRESULT {
    // Only getting data is offered (`DATADIR_GET`).
    if direction != 1 || output.is_null() {
        return E_NOTIMPL;
    }
    // SAFETY: a live data object; the shell copies the formats.
    unsafe {
        let formats: Vec<FORMATETC> = (*this.cast::<Object<DataObjectVtbl, Formats>>())
            .contents
            .iter()
            .map(|(kind, _)| FORMATETC {
                cfFormat: *kind,
                ptd: null_mut(),
                dwAspect: DVASPECT_CONTENT,
                lindex: -1,
                tymed: TYMED_HGLOBAL as u32,
            })
            .collect();
        SHCreateStdEnumFmtEtc(formats.len() as u32, formats.as_ptr(), output)
    }
}

unsafe extern "system" fn advise(
    _: *mut c_void,
    _: *const FORMATETC,
    _: u32,
    _: *mut c_void,
    _: *mut u32,
) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn unadvise(_: *mut c_void, _: u32) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn advisers(_: *mut c_void, _: *mut *mut c_void) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

/// The drag goes on while the left button is held; Escape cancels it.
fn decide(escape: bool, keys: u32) -> HRESULT {
    if escape {
        DRAGDROP_S_CANCEL
    } else if keys & MK_LBUTTON == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn keep_going(_: *mut c_void, escape: i32, keys: u32) -> HRESULT {
    decide(escape != 0, keys)
}

unsafe extern "system" fn feedback(_: *mut c_void, _: u32) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// The clipboard formats and bytes for text a page drags, by web type.
fn formats(items: &[(String, String)]) -> Formats {
    let name = wide("UniformResourceLocatorW");
    // SAFETY: the name is NUL-terminated and outlives the call.
    let link = unsafe { RegisterClipboardFormatW(name.as_ptr()) } as u16;
    let html = crate::clipboard::html_format() as u16;
    let mut found = Formats::new();
    for (kind, value) in items {
        let format = match kind.as_str() {
            "text/plain" => (CF_UNICODETEXT, utf16(value)),
            "text/uri-list" => {
                // The first link of the list; lines starting with `#` are
                // comments.
                let first = value
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && !line.starts_with('#'))
                    .unwrap_or_default();
                (link, utf16(first))
            }
            "text/html" => (html, crate::clipboard::cf_html(value)),
            _ => continue,
        };
        if !found.iter().any(|(existing, _)| *existing == format.0) {
            found.push(format);
        }
    }
    found
}

/// The OLE effects a drag's `effectAllowed` allows.
fn allowed_effects(allowed: &str) -> u32 {
    match allowed {
        "copy" => DROPEFFECT_COPY,
        "move" => DROPEFFECT_MOVE,
        "link" => DROPEFFECT_LINK,
        "copyMove" => DROPEFFECT_COPY | DROPEFFECT_MOVE,
        "copyLink" => DROPEFFECT_COPY | DROPEFFECT_LINK,
        "linkMove" => DROPEFFECT_LINK | DROPEFFECT_MOVE,
        "none" => 0,
        _ => DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
    }
}

/// Drags text, links and HTML (`[web type, data]` pairs) out of the window
/// to other applications until the user drops or cancels; `allowed` is the
/// drag's `effectAllowed`. Returns the effect of the drop, as a
/// `dropEffect`: `none` when nothing took it.
pub fn drag_out(items: &[(String, String)], allowed: &str) -> &'static str {
    let offered = formats(items);
    let effects = allowed_effects(allowed);
    if offered.is_empty() || effects == 0 {
        return "none";
    }
    let data = Box::into_raw(Box::new(Object {
        vtable: &DATA_VTABLE,
        references: AtomicU32::new(1),
        iid: IID_DATA_OBJECT,
        contents: offered,
    }))
    .cast::<c_void>();
    let source = Box::into_raw(Box::new(Object {
        vtable: &SOURCE_VTABLE,
        references: AtomicU32::new(1),
        iid: IID_DROP_SOURCE,
        contents: (),
    }))
    .cast::<c_void>();
    let mut effect = 0;
    // SAFETY: OLE takes its own references to both objects for the drag,
    // which runs its own message loop on this thread; this releases the
    // creator's references afterwards.
    let result = unsafe {
        // OLE tracks the mouse itself.
        ReleaseCapture();
        let result = DoDragDrop(data, source, effects, &mut effect);
        release::<DataObjectVtbl, Formats>(data);
        release::<DropSourceVtbl, ()>(source);
        result
    };
    if result != DRAGDROP_S_DROP {
        return "none";
    }
    if effect & DROPEFFECT_MOVE != 0 {
        "move"
    } else if effect & DROPEFFECT_COPY != 0 {
        "copy"
    } else if effect & DROPEFFECT_LINK != 0 {
        "link"
    } else {
        "none"
    }
}

#[cfg(test)]
mod tests;
