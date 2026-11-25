//! Decodes images GDI+ cannot read, such as WebP, with the Windows Imaging
//! Component's codecs: those Windows has installed for formats browsers
//! show.

#[allow(unused_imports)]
use crate::prelude::*;
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use windows_sys::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows_sys::core::{GUID, HRESULT, IUnknown_Vtbl};

/// `CLSID_WICImagingFactory`.
const FACTORY: GUID = GUID::from_u128(0xcacaf262_9370_4615_a13b_9f5539da4c0a);
/// `IID_IWICImagingFactory`.
const IID_FACTORY: GUID = GUID::from_u128(0xec5ec8a9_c395_4314_9c77_54d7a935ff70);
/// `GUID_WICPixelFormat32bppRGBA`: straight alpha, the order canvas pixels use.
const RGBA: GUID = GUID::from_u128(0xf5c7ad2d_6a8d_43dd_a7a8_a29935261ae9);
/// The largest image decoded, in bytes of pixels.
const MAX_PIXELS_BYTES: usize = 256 * 1024 * 1024;

#[repr(C)]
struct FactoryVtbl {
    base: IUnknown_Vtbl,
    create_decoder_from_filename: usize,
    create_decoder_from_stream: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const GUID,
        u32,
        *mut *mut c_void,
    ) -> HRESULT,
    create_decoder_from_file_handle: usize,
    create_component_info: usize,
    create_decoder: usize,
    create_encoder: usize,
    create_palette: usize,
    create_format_converter: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DecoderVtbl {
    base: IUnknown_Vtbl,
    query_capability: usize,
    initialize: usize,
    get_container_format: usize,
    get_decoder_info: usize,
    copy_palette: usize,
    get_metadata_query_reader: usize,
    get_preview: usize,
    get_color_contexts: usize,
    get_thumbnail: usize,
    get_frame_count: usize,
    get_frame: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
}

/// `IWICBitmapSource`, which frames and format converters extend.
#[repr(C)]
struct SourceVtbl {
    base: IUnknown_Vtbl,
    get_size: unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> HRESULT,
    get_pixel_format: usize,
    get_resolution: usize,
    copy_palette: usize,
    copy_pixels:
        unsafe extern "system" fn(*mut c_void, *const c_void, u32, u32, *mut u8) -> HRESULT,
}

#[repr(C)]
struct ConverterVtbl {
    source: SourceVtbl,
    initialize: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const GUID,
        u32,
        *mut c_void,
        f64,
        u32,
    ) -> HRESULT,
}

/// A COM interface pointer, released when dropped.
struct Com(*mut c_void);

impl Com {
    /// The interface's vtable, viewed as `T`.
    ///
    /// # Safety
    ///
    /// The object must implement the interface `T` describes.
    unsafe fn vtable<T>(&self) -> &T {
        // SAFETY: a COM object starts with its vtable pointer.
        unsafe { &**(self.0 as *const *const T) }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: this holds one reference to a live object.
        unsafe { (self.vtable::<IUnknown_Vtbl>().Release)(self.0) };
    }
}

/// Runs a COM call that creates an object, which the result then owns.
fn create(call: impl FnOnce(*mut *mut c_void) -> HRESULT) -> Option<Com> {
    let mut object = null_mut();
    (call(&mut object) >= 0 && !object.is_null()).then(|| Com(object))
}

/// The size and straight-alpha RGBA pixels of the first frame of an image
/// in a format a WIC codec installed on this system reads; `None` when none
/// does. COM must be initialized on the calling thread.
pub(crate) fn decode(stream: *mut c_void) -> Option<((u32, u32), Vec<u8>)> {
    // SAFETY: the GUIDs are valid and the out pointer writable.
    let factory = create(|out| unsafe {
        CoCreateInstance(
            &FACTORY,
            null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_FACTORY,
            out,
        )
    })?;
    // SAFETY: each object implements the interface its vtable describes, the
    // stream is live, and the pixel buffer holds `stride * height` bytes.
    unsafe {
        let factory_table = factory.vtable::<FactoryVtbl>();
        let decoder = create(|out| {
            (factory_table.create_decoder_from_stream)(factory.0, stream, null(), 0, out)
        })?;
        let frame = create(|out| (decoder.vtable::<DecoderVtbl>().get_frame)(decoder.0, 0, out))?;
        let converter = create(|out| (factory_table.create_format_converter)(factory.0, out))?;
        let table = converter.vtable::<ConverterVtbl>();
        if (table.initialize)(converter.0, frame.0, &RGBA, 0, null_mut(), 0.0, 0) < 0 {
            return None;
        }
        let (mut width, mut height) = (0, 0);
        if (table.source.get_size)(converter.0, &mut width, &mut height) < 0
            || width == 0
            || height == 0
        {
            return None;
        }
        let stride = width.checked_mul(4)?;
        let length = (stride as usize).checked_mul(height as usize)?;
        if length > MAX_PIXELS_BYTES {
            return None;
        }
        let mut pixels = vec![0_u8; length];
        ((table.source.copy_pixels)(
            converter.0,
            null(),
            stride,
            length as u32,
            pixels.as_mut_ptr(),
        ) >= 0)
            .then_some(((width, height), pixels))
    }
}
