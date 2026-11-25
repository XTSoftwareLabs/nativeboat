//! What Electron's `nativeImage` reads and writes through Windows, with the
//! calls Chromium makes: JPEG through the Windows Imaging Component, `.ico`
//! files through `LoadImage`, and Explorer's thumbnails through the local
//! thumbnail cache, whose icons are read as `IconUtil` reads them.

use crate::com::{Com, Guid, Hresult, S_OK};
#[allow(unused_imports)]
use crate::prelude::*;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::{null, null_mut};

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid {
        data1,
        data2,
        data3,
        data4,
    }
}

const CLSID_WIC_FACTORY: Guid = guid(
    0xcacaf262,
    0x9370,
    0x4615,
    [0xa1, 0x3b, 0x9f, 0x55, 0x39, 0xda, 0x4c, 0x0a],
);
const IID_WIC_FACTORY: Guid = guid(
    0xec5ec8a9,
    0xc395,
    0x4314,
    [0x9c, 0x77, 0x54, 0xd7, 0xa9, 0x35, 0xff, 0x70],
);
const CONTAINER_JPEG: Guid = guid(
    0x19e4a5aa,
    0x5662,
    0x4fc5,
    [0xa0, 0xc0, 0x17, 0x58, 0x02, 0x8e, 0x10, 0x57],
);
/// `GUID_WICPixelFormat32bppPBGRA`: premultiplied BGRA, as Skia keeps
/// pixels on Windows.
const PBGRA: Guid = guid(
    0x6fddc324,
    0x4e03,
    0x4bfe,
    [0xb1, 0x85, 0x3d, 0x77, 0x76, 0x8d, 0xc9, 0x10],
);
/// `GUID_WICPixelFormat24bppBGR`.
const BGR: Guid = guid(
    0x6fddc324,
    0x4e03,
    0x4bfe,
    [0xb1, 0x85, 0x3d, 0x77, 0x76, 0x8d, 0xc9, 0x0c],
);
const CLSID_LOCAL_THUMBNAIL_CACHE: Guid = guid(
    0x50ef4544,
    0xac9f,
    0x4a8e,
    [0xb2, 0x1b, 0x8a, 0x26, 0x18, 0x0d, 0xb1, 0x3f],
);
const IID_THUMBNAIL_CACHE: Guid = guid(
    0xf676c15d,
    0x596a,
    0x4ce2,
    [0x82, 0x34, 0x33, 0x99, 0x6f, 0x44, 0x5d, 0xb1],
);
const IID_SHELL_ITEM: Guid = guid(
    0x43826d1e,
    0xe718,
    0x42ee,
    [0xbc, 0x55, 0xa1, 0xe2, 0x61, 0xc3, 0x7b, 0xfe],
);

// Vtable slots. IWICImagingFactory:
const CREATE_DECODER_FROM_STREAM: usize = 4;
const CREATE_ENCODER: usize = 8;
const CREATE_FORMAT_CONVERTER: usize = 10;
// IWICBitmapDecoder:
const GET_CONTAINER_FORMAT: usize = 5;
const GET_FRAME: usize = 13;
// IWICBitmapSource, which frames and converters extend:
const GET_SIZE: usize = 3;
const COPY_PIXELS: usize = 7;
// IWICFormatConverter:
const CONVERTER_INITIALIZE: usize = 8;
// IWICBitmapEncoder:
const ENCODER_INITIALIZE: usize = 3;
const CREATE_NEW_FRAME: usize = 10;
const ENCODER_COMMIT: usize = 11;
// IWICBitmapFrameEncode:
const FRAME_INITIALIZE: usize = 3;
const SET_SIZE: usize = 4;
const SET_PIXEL_FORMAT: usize = 6;
const WRITE_PIXELS: usize = 10;
const FRAME_COMMIT: usize = 12;
// IPropertyBag2:
const PROPERTY_BAG_WRITE: usize = 4;
// IStream:
const STREAM_READ: usize = 3;
const STREAM_SEEK: usize = 5;
// IThumbnailCache:
const GET_THUMBNAIL: usize = 3;
// ISharedBitmap:
const GET_SHARED_BITMAP: usize = 3;

const COINIT_APARTMENTTHREADED: u32 = 2;
const WIC_DECODE_METADATA_CACHE_ON_DEMAND: u32 = 0;
const WIC_BITMAP_ENCODER_NO_CACHE: u32 = 2;
const WIC_BITMAP_DITHER_TYPE_NONE: u32 = 0;
const WIC_BITMAP_PALETTE_TYPE_CUSTOM: u32 = 0;
const STREAM_SEEK_SET: u32 = 0;
const STREAM_SEEK_CUR: u32 = 1;
const VT_R4: u16 = 4;
const PROPBAG2_TYPE_DATA: u32 = 1;
const WTS_SCALETOREQUESTEDSIZE: u32 = 0x40;
const WTS_SCALEUP: u32 = 0x1_0000;
const IMAGE_ICON: u32 = 1;
const LR_LOADFROMFILE: u32 = 0x10;
const DI_MASK: u32 = 1;
const DI_NORMAL: u32 = 3;
const BI_BITFIELDS: u32 = 3;
const DIB_RGB_COLORS: u32 = 0;
/// The largest picture read, in pixels, as the PNG decoder allows.
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

#[repr(C)]
struct PropertyBagEntry {
    kind: u32,
    variant_type: u16,
    clipboard_format: u16,
    hint: u32,
    name: *const u16,
    class: Guid,
}

#[repr(C)]
struct Variant {
    variant_type: u16,
    reserved: [u16; 3],
    value: f32,
    padding: [u32; 3],
}

#[repr(C)]
struct IconInfo {
    icon: i32,
    hotspot_x: u32,
    hotspot_y: u32,
    mask: *mut c_void,
    color: *mut c_void,
}

#[repr(C)]
#[derive(Default)]
struct Bitmap {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_per_pixel: u16,
    bits: usize,
}

/// `BITMAPV5HEADER`, as `IconUtil` describes its drawing surface.
#[repr(C)]
#[derive(Default)]
struct BitmapHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pixels_per_meter: i32,
    y_pixels_per_meter: i32,
    colors_used: u32,
    colors_important: u32,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    alpha_mask: u32,
    color_space: u32,
    endpoints: [i32; 9],
    gamma_red: u32,
    gamma_green: u32,
    gamma_blue: u32,
    intent: u32,
    profile_data: u32,
    profile_size: u32,
    reserved: u32,
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, model: u32) -> Hresult;
    fn CoUninitialize();
    fn CreateStreamOnHGlobal(global: *mut c_void, delete: i32, stream: *mut *mut c_void)
    -> Hresult;
}

#[link(name = "shlwapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHCreateMemStream(data: *const u8, size: u32) -> *mut c_void;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHCreateItemFromParsingName(
        path: *const u16,
        context: *mut c_void,
        iid: *const Guid,
        item: *mut *mut c_void,
    ) -> Hresult;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadImageW(
        instance: *mut c_void,
        name: *const u16,
        kind: u32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> *mut c_void;
    fn GetIconInfo(icon: *mut c_void, info: *mut IconInfo) -> i32;
    fn CreateIconIndirect(info: *const IconInfo) -> *mut c_void;
    fn DestroyIcon(icon: *mut c_void) -> i32;
    fn DrawIconEx(
        dc: *mut c_void,
        x: i32,
        y: i32,
        icon: *mut c_void,
        width: i32,
        height: i32,
        step: u32,
        brush: *mut c_void,
        flags: u32,
    ) -> i32;
    fn GetDC(window: *mut c_void) -> *mut c_void;
    fn ReleaseDC(window: *mut c_void, dc: *mut c_void) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetObjectW(object: *mut c_void, size: i32, out: *mut c_void) -> i32;
    fn CreateDIBSection(
        dc: *mut c_void,
        info: *const BitmapHeader,
        usage: u32,
        bits: *mut *mut c_void,
        section: *mut c_void,
        offset: u32,
    ) -> *mut c_void;
    fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
    fn SelectObject(dc: *mut c_void, object: *mut c_void) -> *mut c_void;
    fn DeleteObject(object: *mut c_void) -> i32;
    fn DeleteDC(dc: *mut c_void) -> i32;
}

/// A picture as Skia keeps it: its size and premultiplied BGRA pixels.
pub(crate) type Pixels = (u32, u32, Vec<u8>);

/// Runs `work` with COM available on this thread, joining an apartment
/// only when the thread has none.
fn with_com<T>(work: impl FnOnce() -> T) -> T {
    // SAFETY: balanced below; a thread already in an apartment keeps it.
    let joined = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED) } >= 0;
    let result = work();
    if joined {
        // SAFETY: balances the `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
    result
}

/// An interface a method gives through its out pointer.
fn created(make: impl FnOnce(*mut *mut c_void) -> Hresult) -> Option<Com> {
    let mut object = null_mut();
    (make(&mut object) >= 0 && !object.is_null()).then(|| Com(object))
}

fn factory() -> Option<Com> {
    Com::create(&CLSID_WIC_FACTORY, &IID_WIC_FACTORY)
}

/// The pixels of a JPEG file, as Chromium's JPEG codec reads them; other
/// formats are not read.
pub(crate) fn decode_jpeg(bytes: &[u8]) -> Option<Pixels> {
    let size = u32::try_from(bytes.len()).ok()?;
    with_com(|| {
        let factory = factory()?;
        // SAFETY: the stream copies the bytes; the out pointers are writable
        // and each interface is released when dropped.
        unsafe {
            let stream = SHCreateMemStream(bytes.as_ptr(), size);
            if stream.is_null() {
                return None;
            }
            let stream = Com(stream);
            let decoder = created(|out| {
                factory.method::<unsafe extern "system" fn(
                    *mut c_void,
                    *mut c_void,
                    *const Guid,
                    u32,
                    *mut *mut c_void,
                ) -> Hresult>(CREATE_DECODER_FROM_STREAM)(
                    factory.0,
                    stream.0,
                    null(),
                    WIC_DECODE_METADATA_CACHE_ON_DEMAND,
                    out,
                )
            })?;
            let mut container = guid(0, 0, 0, [0; 8]);
            decoder.method::<unsafe extern "system" fn(*mut c_void, *mut Guid) -> Hresult>(
                GET_CONTAINER_FORMAT,
            )(decoder.0, &mut container);
            if !same(&container, &CONTAINER_JPEG) {
                return None;
            }
            let frame = created(|out| {
                decoder.method::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> Hresult>(
                    GET_FRAME,
                )(decoder.0, 0, out)
            })?;
            let converter = created(|out| {
                factory
                    .method::<unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult>(
                        CREATE_FORMAT_CONVERTER,
                    )(factory.0, out)
            })?;
            let initialized = converter.method::<unsafe extern "system" fn(
                *mut c_void,
                *mut c_void,
                *const Guid,
                u32,
                *mut c_void,
                f64,
                u32,
            ) -> Hresult>(CONVERTER_INITIALIZE)(
                converter.0,
                frame.0,
                &PBGRA,
                WIC_BITMAP_DITHER_TYPE_NONE,
                null_mut(),
                0.0,
                WIC_BITMAP_PALETTE_TYPE_CUSTOM,
            );
            if initialized < 0 {
                return None;
            }
            let (mut width, mut height) = (0_u32, 0_u32);
            converter
                .method::<unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> Hresult>(
                    GET_SIZE,
                )(converter.0, &mut width, &mut height);
            if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
                return None;
            }
            let mut pixels = vec![0_u8; width as usize * height as usize * 4];
            let copied = converter.method::<unsafe extern "system" fn(
                *mut c_void,
                *const c_void,
                u32,
                u32,
                *mut u8,
            ) -> Hresult>(COPY_PIXELS)(
                converter.0,
                null(),
                width * 4,
                pixels.len() as u32,
                pixels.as_mut_ptr(),
            );
            (copied >= 0).then_some((width, height, pixels))
        }
    })
}

fn same(left: &Guid, right: &Guid) -> bool {
    (left.data1, left.data2, left.data3, left.data4)
        == (right.data1, right.data2, right.data3, right.data4)
}

/// Premultiplied BGRA pixels as a JPEG file of `quality` from 0 to 100,
/// subsampled 4:2:0 with the standard tables, as Chromium encodes them;
/// colours are taken as stored, over black.
pub(crate) fn encode_jpeg(width: u32, height: u32, pixels: &[u8], quality: u32) -> Option<Vec<u8>> {
    let stride = (width as usize * 3).div_ceil(4) * 4;
    let mut rows = vec![0_u8; stride * height as usize];
    for (row, line) in rows
        .chunks_exact_mut(stride)
        .zip(pixels.chunks_exact(width as usize * 4))
    {
        for (out, pixel) in row.chunks_exact_mut(3).zip(line.chunks_exact(4)) {
            out.copy_from_slice(&pixel[..3]);
        }
    }
    with_com(|| {
        let factory = factory()?;
        // SAFETY: as in `decode_jpeg`; the property name outlives the call
        // that reads it.
        unsafe {
            let stream = created(|out| CreateStreamOnHGlobal(null_mut(), 1, out))?;
            let encoder = created(|out| {
                factory.method::<unsafe extern "system" fn(
                    *mut c_void,
                    *const Guid,
                    *const Guid,
                    *mut *mut c_void,
                ) -> Hresult>(CREATE_ENCODER)(
                    factory.0, &CONTAINER_JPEG, null(), out
                )
            })?;
            let ok = |result: Hresult| (result >= 0).then_some(());
            ok(encoder.method::<unsafe extern "system" fn(
                *mut c_void,
                *mut c_void,
                u32,
            ) -> Hresult>(ENCODER_INITIALIZE)(
                encoder.0,
                stream.0,
                WIC_BITMAP_ENCODER_NO_CACHE,
            ))?;
            let mut options = null_mut();
            let frame = created(|out| {
                encoder.method::<unsafe extern "system" fn(
                    *mut c_void,
                    *mut *mut c_void,
                    *mut *mut c_void,
                ) -> Hresult>(CREATE_NEW_FRAME)(encoder.0, out, &mut options)
            })?;
            let options = Com(options);
            let name: Vec<u16> = "ImageQuality".encode_utf16().chain([0]).collect();
            let entry = PropertyBagEntry {
                kind: PROPBAG2_TYPE_DATA,
                variant_type: VT_R4,
                clipboard_format: 0,
                hint: 0,
                name: name.as_ptr(),
                class: guid(0, 0, 0, [0; 8]),
            };
            let value = Variant {
                variant_type: VT_R4,
                reserved: [0; 3],
                value: quality as f32 / 100.0,
                padding: [0; 3],
            };
            ok(options.method::<unsafe extern "system" fn(
                *mut c_void,
                u32,
                *const PropertyBagEntry,
                *const Variant,
            ) -> Hresult>(PROPERTY_BAG_WRITE)(
                options.0, 1, &entry, &value
            ))?;
            ok(frame.method::<unsafe extern "system" fn(
                *mut c_void,
                *mut c_void,
            ) -> Hresult>(FRAME_INITIALIZE)(
                frame.0, options.0
            ))?;
            ok(frame.method::<unsafe extern "system" fn(
                *mut c_void,
                u32,
                u32,
            ) -> Hresult>(SET_SIZE)(frame.0, width, height))?;
            let mut format = guid(BGR.data1, BGR.data2, BGR.data3, BGR.data4);
            ok(frame.method::<unsafe extern "system" fn(
                *mut c_void,
                *mut Guid,
            ) -> Hresult>(SET_PIXEL_FORMAT)(
                frame.0, &mut format
            ))?;
            if !same(&format, &BGR) {
                return None;
            }
            ok(frame.method::<unsafe extern "system" fn(
                *mut c_void,
                u32,
                u32,
                u32,
                *const u8,
            ) -> Hresult>(WRITE_PIXELS)(
                frame.0,
                height,
                stride as u32,
                rows.len() as u32,
                rows.as_ptr(),
            ))?;
            ok(frame
                .method::<unsafe extern "system" fn(*mut c_void) -> Hresult>(
                    FRAME_COMMIT,
                )(frame.0))?;
            ok(encoder
                .method::<unsafe extern "system" fn(*mut c_void) -> Hresult>(
                    ENCODER_COMMIT,
                )(encoder.0))?;
            read_stream(&stream)
        }
    })
}

/// What a stream holds before its current position.
fn read_stream(stream: &Com) -> Option<Vec<u8>> {
    type Seek = unsafe extern "system" fn(*mut c_void, i64, u32, *mut u64) -> Hresult;
    let mut length = 0_u64;
    // SAFETY: a live stream; the buffer holds `length` bytes.
    unsafe {
        if stream.method::<Seek>(STREAM_SEEK)(stream.0, 0, STREAM_SEEK_CUR, &mut length) < 0
            || stream.method::<Seek>(STREAM_SEEK)(stream.0, 0, STREAM_SEEK_SET, null_mut()) < 0
        {
            return None;
        }
        let mut bytes = vec![0_u8; usize::try_from(length).ok()?];
        let mut read = 0_u32;
        let result = stream.method::<unsafe extern "system" fn(
            *mut c_void,
            *mut u8,
            u32,
            *mut u32,
        ) -> Hresult>(STREAM_READ)(
            stream.0, bytes.as_mut_ptr(), bytes.len() as u32, &mut read
        );
        (result >= 0 && read as usize == bytes.len()).then_some(bytes)
    }
}

fn wide(path: &str) -> Vec<u16> {
    path.encode_utf16().chain([0]).collect()
}

/// An `.ico` file's icon as `LoadImage` scales it to `size` pixels square,
/// as Electron reads icon files.
pub(crate) fn load_icon(path: &str, size: i32) -> Option<Pixels> {
    let name = wide(path);
    // SAFETY: a null-terminated name; the icon is destroyed below.
    let icon = unsafe {
        LoadImageW(
            null_mut(),
            name.as_ptr(),
            IMAGE_ICON,
            size,
            size,
            LR_LOADFROMFILE,
        )
    };
    if icon.is_null() {
        return None;
    }
    let pixels = icon_pixels(icon);
    // SAFETY: the icon `LoadImageW` created.
    unsafe { DestroyIcon(icon) };
    pixels
}

/// The thumbnail Explorer shows of a file, scaled up or down to `size` pixels
/// on its longer side, as Electron's `createThumbnailFromPath` reads it on
/// Windows; or why there is none, in Electron's words.
pub(crate) fn thumbnail(path: &str, size: u32) -> Result<Pixels, &'static str> {
    let name = wide(path);
    with_com(|| {
        // SAFETY: a null-terminated name; the out pointers are writable and
        // each interface is released when dropped. The shared bitmap stays
        // its cache's.
        unsafe {
            let item = created(|out| {
                SHCreateItemFromParsingName(name.as_ptr(), null_mut(), &IID_SHELL_ITEM, out)
            })
            .ok_or("Failed to create IShellItem from the given path")?;
            let cache = Com::create(&CLSID_LOCAL_THUMBNAIL_CACHE, &IID_THUMBNAIL_CACHE)
                .ok_or("Failed to acquire local thumbnail cache reference")?;
            let mut flags = 0_u32;
            let mut id = [0_u8; 16];
            let shared = created(|out| {
                cache.method::<unsafe extern "system" fn(
                    *mut c_void,
                    *mut c_void,
                    u32,
                    u32,
                    *mut *mut c_void,
                    *mut u32,
                    *mut u8,
                ) -> Hresult>(GET_THUMBNAIL)(
                    cache.0,
                    item.0,
                    size,
                    WTS_SCALETOREQUESTEDSIZE | WTS_SCALEUP,
                    out,
                    &mut flags,
                    id.as_mut_ptr(),
                )
            })
            .ok_or("Failed to get thumbnail from local thumbnail cache reference")?;
            let mut bitmap = null_mut();
            if shared.method::<unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult>(
                GET_SHARED_BITMAP,
            )(shared.0, &mut bitmap)
                < 0
            {
                return Err("Failed to extract bitmap from thumbnail");
            }
            let mut info = Bitmap::default();
            if GetObjectW(bitmap, size_of::<Bitmap>() as i32, (&raw mut info).cast()) == 0 {
                return Err("Could not convert HBITMAP to BITMAP");
            }
            let icon = CreateIconIndirect(&IconInfo {
                icon: 1,
                hotspot_x: 0,
                hotspot_y: 0,
                mask: bitmap,
                color: bitmap,
            });
            let pixels = icon_pixels(icon);
            if !icon.is_null() {
                DestroyIcon(icon);
            }
            Ok(pixels.unwrap_or((0, 0, Vec::new())))
        }
    })
}

/// An icon's pixels as Chromium's `IconUtil::CreateSkBitmapFromHICON`
/// reads them: drawn onto transparent black, with the AND mask giving
/// opacity when the colours carry no alpha.
fn icon_pixels(icon: *mut c_void) -> Option<Pixels> {
    let mut info = IconInfo {
        icon: 0,
        hotspot_x: 0,
        hotspot_y: 0,
        mask: null_mut(),
        color: null_mut(),
    };
    // SAFETY: a live icon; the bitmaps `GetIconInfo` makes are deleted
    // below, and the drawing surface and its context are released.
    unsafe {
        if icon.is_null() || GetIconInfo(icon, &mut info) == 0 {
            return None;
        }
        let mut mask = Bitmap::default();
        let described = GetObjectW(
            info.mask,
            size_of::<Bitmap>() as i32,
            (&raw mut mask).cast(),
        );
        let has_color = !info.color.is_null();
        for bitmap in [info.mask, info.color] {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
        }
        // A monochrome icon's mask holds its AND and XOR masks, one above
        // the other.
        let width = mask.width;
        let height = mask.height / if has_color { 1 } else { 2 };
        if described == 0 || width <= 0 || height <= 0 {
            return None;
        }
        let header = BitmapHeader {
            size: size_of::<BitmapHeader>() as u32,
            width,
            height: -height,
            planes: 1,
            bit_count: 32,
            compression: BI_BITFIELDS,
            red_mask: 0x00ff_0000,
            green_mask: 0x0000_ff00,
            blue_mask: 0x0000_00ff,
            alpha_mask: 0xff00_0000,
            ..BitmapHeader::default()
        };
        let screen = GetDC(null_mut());
        let mut bits = null_mut();
        let surface = CreateDIBSection(screen, &header, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let context = CreateCompatibleDC(screen);
        ReleaseDC(null_mut(), screen);
        if surface.is_null() || context.is_null() {
            for (object, delete) in [
                (surface, DeleteObject as unsafe extern "system" fn(_) -> _),
                (context, DeleteDC),
            ] {
                if !object.is_null() {
                    delete(object);
                }
            }
            return None;
        }
        let previous = SelectObject(context, surface);
        let count = width as usize * height as usize;
        let pixels = core::slice::from_raw_parts_mut(bits.cast::<u32>(), count);
        pixels.fill(0);
        DrawIconEx(context, 0, 0, icon, width, height, 0, null_mut(), DI_MASK);
        let opaque: Vec<bool> = pixels.iter().map(|pixel| *pixel == 0).collect();
        pixels.fill(0);
        DrawIconEx(context, 0, 0, icon, width, height, 0, null_mut(), DI_NORMAL);
        let mut colours = pixels.to_vec();
        SelectObject(context, previous);
        DeleteObject(surface);
        DeleteDC(context);
        if colours.iter().all(|pixel| pixel >> 24 == 0) {
            for (pixel, opaque) in colours.iter_mut().zip(opaque) {
                if opaque {
                    *pixel |= 0xff00_0000;
                } else {
                    *pixel &= 0x00ff_ffff;
                }
            }
        }
        let bytes = colours
            .iter()
            .flat_map(|pixel| pixel.to_le_bytes())
            .collect();
        Some((width as u32, height as u32, bytes))
    }
}

#[allow(dead_code)]
const _: Hresult = S_OK;
