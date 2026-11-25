//! What a page's `<video>` and `<audio>` elements show before playback:
//! the duration of their media and the first frame of its video, read with
//! Media Foundation, which decodes what Windows has codecs for.

use crate::com::{Com, Guid, Hresult};
#[allow(unused_imports)]
use crate::prelude::*;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::null_mut;

/// Where a media element's media comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaSource {
    /// A file on disk.
    Path(String),
    /// An http(s) URL, which Media Foundation downloads as it reads.
    Url(String),
    /// Media already in memory, such as a `data:` URL's.
    Bytes(Vec<u8>),
}

/// A video's first frame, upright: its size and straight RGBA pixels.
#[derive(Debug, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// What a media element knows once its media's metadata and first frame
/// are loaded.
#[derive(Debug, PartialEq)]
pub struct Media {
    /// The duration in seconds; infinite for a live stream, NaN unknown.
    pub duration: f64,
    /// The first frame, for media with video.
    pub frame: Option<Frame>,
}

const MF_VERSION: u32 = 0x0002_0070;
const COINIT_MULTITHREADED: u32 = 0;
const FIRST_VIDEO_STREAM: u32 = 0xFFFF_FFFC;
const ALL_STREAMS: u32 = 0xFFFF_FFFE;
const MEDIA_SOURCE: u32 = 0xFFFF_FFFF;
const READ_ERROR: u32 = 0x1;
const END_OF_STREAM: u32 = 0x2;
const VT_UI8: u16 = 21;

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid {
        data1,
        data2,
        data3,
        data4,
    }
}

/// Media types' base GUID, after the first field.
const fn media_subtype(data1: u32) -> Guid {
    guid(
        data1,
        0x0000,
        0x0010,
        [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
    )
}

const MF_MT_MAJOR_TYPE: Guid = guid(
    0x48eb_a18e,
    0xf8c9,
    0x4687,
    [0xbf, 0x11, 0x0a, 0x74, 0xc9, 0xf9, 0x6a, 0x8f],
);
const MF_MT_SUBTYPE: Guid = guid(
    0xf7e3_4c9a,
    0x42e8,
    0x4714,
    [0xb7, 0x4b, 0xcb, 0x29, 0xd7, 0x2c, 0x35, 0xe5],
);
const MF_MT_FRAME_SIZE: Guid = guid(
    0x1652_c33d,
    0xd6b2,
    0x4012,
    [0xb8, 0x34, 0x72, 0x03, 0x08, 0x49, 0xa3, 0x7d],
);
const MF_MT_DEFAULT_STRIDE: Guid = guid(
    0x644b_4e48,
    0x1e02,
    0x4516,
    [0xb0, 0xeb, 0xc0, 0x1c, 0xa9, 0xd4, 0x9a, 0xc6],
);
const MF_MT_MINIMUM_DISPLAY_APERTURE: Guid = guid(
    0xd738_8766,
    0x18fe,
    0x48c6,
    [0xa1, 0x77, 0xee, 0x89, 0x48, 0x67, 0xc8, 0xc4],
);
const MF_MT_VIDEO_ROTATION: Guid = guid(
    0xc380_465d,
    0x2271,
    0x428c,
    [0x9b, 0x83, 0xec, 0xea, 0x3b, 0x4a, 0x85, 0xc1],
);
const MF_PD_DURATION: Guid = guid(
    0x6c99_0d33,
    0xbb8e,
    0x477a,
    [0x85, 0x98, 0x0d, 0x5d, 0x96, 0xfc, 0xd8, 0x8a],
);
const ENABLE_VIDEO_PROCESSING: Guid = guid(
    0xfb39_4f3d,
    0xccf1,
    0x42ee,
    [0xbb, 0xb3, 0xf9, 0xb8, 0x45, 0xd5, 0x68, 0x1d],
);
const MF_MT_YUV_MATRIX: Guid = guid(
    0x3e23_d450,
    0x2c75,
    0x4d25,
    [0xa0, 0x0e, 0xb9, 0x16, 0x70, 0xd1, 0x23, 0x27],
);
const MF_MT_VIDEO_NOMINAL_RANGE: Guid = guid(
    0xc21b_8ee5,
    0xb956,
    0x4071,
    [0x8d, 0xaf, 0x32, 0x5e, 0xdf, 0x5c, 0xab, 0x11],
);
const MF_MT_VIDEO_PRIMARIES: Guid = guid(
    0xdbfb_e4d7,
    0x0740,
    0x4ee0,
    [0x81, 0x92, 0x85, 0x0a, 0xb0, 0xe2, 0x19, 0x35],
);
const MEDIA_TYPE_VIDEO: Guid = media_subtype(0x7364_6976);
const VIDEO_FORMAT_NV12: Guid = media_subtype(0x3231_564e);

// Vtable slots: IMFAttributes, which media types extend; IMFSourceReader;
// IMFSample; IMFMediaBuffer.
const GET_UINT32: usize = 7;
const GET_UINT64: usize = 8;
const GET_BLOB: usize = 15;
const SET_UINT32: usize = 21;
const SET_GUID: usize = 24;
const SET_STREAM_SELECTION: usize = 4;
const GET_CURRENT_MEDIA_TYPE: usize = 6;
const SET_CURRENT_MEDIA_TYPE: usize = 7;
const READ_SAMPLE: usize = 9;
const GET_PRESENTATION_ATTRIBUTE: usize = 12;
const CONVERT_TO_CONTIGUOUS_BUFFER: usize = 41;
const LOCK: usize = 3;
const UNLOCK: usize = 4;

#[link(name = "mfplat", kind = "raw-dylib")]
unsafe extern "system" {
    fn MFStartup(version: u32, flags: u32) -> Hresult;
    fn MFShutdown() -> Hresult;
    fn MFCreateAttributes(attributes: *mut *mut c_void, size: u32) -> Hresult;
    fn MFCreateMediaType(media_type: *mut *mut c_void) -> Hresult;
    fn MFCreateMFByteStreamOnStream(stream: *mut c_void, byte_stream: *mut *mut c_void) -> Hresult;
}

#[link(name = "mfreadwrite", kind = "raw-dylib")]
unsafe extern "system" {
    fn MFCreateSourceReaderFromURL(
        url: *const u16,
        attributes: *mut c_void,
        reader: *mut *mut c_void,
    ) -> Hresult;
    fn MFCreateSourceReaderFromByteStream(
        byte_stream: *mut c_void,
        attributes: *mut c_void,
        reader: *mut *mut c_void,
    ) -> Hresult;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, model: u32) -> Hresult;
    fn CoUninitialize();
}

#[link(name = "shlwapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHCreateMemStream(data: *const u8, size: u32) -> *mut c_void;
}

/// An interface a creation function returns through its out pointer.
fn created(create: impl FnOnce(*mut *mut c_void) -> Hresult) -> Option<Com> {
    let mut object = null_mut();
    (create(&mut object) >= 0 && !object.is_null()).then(|| Com(object))
}

/// Reads media's duration and first video frame; `None` when Windows cannot
/// read it. Runs on any thread: it joins the multithreaded apartment while
/// it reads.
pub fn read(source: &MediaSource) -> Option<Media> {
    // SAFETY: balanced below; a thread already in another apartment keeps it.
    let joined = unsafe { CoInitializeEx(null_mut(), COINIT_MULTITHREADED) } >= 0;
    // SAFETY: balanced by `MFShutdown`.
    let started = unsafe { MFStartup(MF_VERSION, 0) } >= 0;
    let media = if started { read_started(source) } else { None };
    if started {
        // SAFETY: balances the `MFStartup` above.
        unsafe { MFShutdown() };
    }
    if joined {
        // SAFETY: balances the `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
    media
}

fn read_started(source: &MediaSource) -> Option<Media> {
    // The reader converts decoded video to NV12, whatever the decoder gives.
    // SAFETY: the out pointer is writable.
    let attributes = created(|out| unsafe { MFCreateAttributes(out, 1) })?;
    // SAFETY: SetUINT32(key, value) on a live attribute store.
    unsafe {
        attributes.method::<unsafe extern "system" fn(*mut c_void, *const Guid, u32) -> Hresult>(
            SET_UINT32,
        )(attributes.0, &ENABLE_VIDEO_PROCESSING, 1)
    };
    let reader = match source {
        MediaSource::Path(path) | MediaSource::Url(path) => {
            let url: Vec<u16> = path.encode_utf16().chain([0]).collect();
            // SAFETY: a null-terminated URL and a live attribute store.
            created(|out| unsafe { MFCreateSourceReaderFromURL(url.as_ptr(), attributes.0, out) })?
        }
        MediaSource::Bytes(bytes) => {
            let size = u32::try_from(bytes.len()).ok()?;
            // SAFETY: the bytes are copied into the new stream.
            let stream = unsafe { SHCreateMemStream(bytes.as_ptr(), size) };
            if stream.is_null() {
                return None;
            }
            let stream = Com(stream);
            // SAFETY: a live stream; the byte stream keeps its own reference.
            let byte_stream =
                created(|out| unsafe { MFCreateMFByteStreamOnStream(stream.0, out) })?;
            // SAFETY: a live byte stream and attribute store.
            created(|out| unsafe {
                MFCreateSourceReaderFromByteStream(byte_stream.0, attributes.0, out)
            })?
        }
    };
    let duration = duration(&reader);
    Some(Media {
        duration,
        frame: first_frame(&reader),
    })
}

/// The media's duration in seconds, from its presentation descriptor.
fn duration(reader: &Com) -> f64 {
    // A PROPVARIANT: its type, then its value after 8 bytes.
    let mut value = [0u64; 3];
    // SAFETY: GetPresentationAttribute(stream, key, value) writes a
    // PROPVARIANT; a duration holds no memory to free.
    let found = unsafe {
        reader.method::<unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut u64) -> Hresult>(
            GET_PRESENTATION_ATTRIBUTE,
        )(reader.0, MEDIA_SOURCE, &MF_PD_DURATION, value.as_mut_ptr())
    } >= 0;
    if found && value[0] as u16 == VT_UI8 {
        // In units of 100 nanoseconds.
        value[1] as f64 / 10_000_000.0
    } else {
        f64::NAN
    }
}

/// A media type attribute, read as a 32- or 64-bit number.
fn attribute_u32(media_type: &Com, key: &Guid) -> Option<u32> {
    let mut value = 0u32;
    // SAFETY: GetUINT32(key, value) on a live media type.
    let result = unsafe {
        media_type
            .method::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32) -> Hresult>(
                GET_UINT32,
            )(media_type.0, key, &mut value)
    };
    (result >= 0).then_some(value)
}

fn attribute_u64(media_type: &Com, key: &Guid) -> Option<u64> {
    let mut value = 0u64;
    // SAFETY: GetUINT64(key, value) on a live media type.
    let result = unsafe {
        media_type
            .method::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut u64) -> Hresult>(
                GET_UINT64,
            )(media_type.0, key, &mut value)
    };
    (result >= 0).then_some(value)
}

/// The part of the decoded picture that is shown, as x, y, width and
/// height: decoders pad frames, such as 1080 lines to 1088.
fn aperture(media_type: &Com) -> Option<(u32, u32, u32, u32)> {
    // An MFVideoArea: fixed-point offsets, each a 16-bit fraction and a
    // 16-bit integer, then the area's width and height.
    let mut area = [0i32; 4];
    let mut size = 0u32;
    // SAFETY: GetBlob(key, buffer, size, written) into a 16-byte buffer.
    let result = unsafe {
        media_type.method::<unsafe extern "system" fn(
            *mut c_void,
            *const Guid,
            *mut u8,
            u32,
            *mut u32,
        ) -> Hresult>(GET_BLOB)(
            media_type.0,
            &MF_MT_MINIMUM_DISPLAY_APERTURE,
            area.as_mut_ptr().cast(),
            16,
            &mut size,
        )
    };
    if result < 0 || size != 16 {
        return None;
    }
    let offset = |fixed: i32| u32::try_from(fixed >> 16).ok();
    Some((
        offset(area[0])?,
        offset(area[1])?,
        u32::try_from(area[2]).ok()?,
        u32::try_from(area[3]).ok()?,
    ))
}

/// The first frame of the media's first video stream, upright.
fn first_frame(reader: &Com) -> Option<Frame> {
    type Selection = unsafe extern "system" fn(*mut c_void, u32, i32) -> Hresult;
    // Only the video is read.
    // SAFETY: SetStreamSelection(stream, selected) on a live reader.
    unsafe {
        let select = reader.method::<Selection>(SET_STREAM_SELECTION);
        select(reader.0, ALL_STREAMS, 0);
        if select(reader.0, FIRST_VIDEO_STREAM, 1) < 0 {
            return None;
        }
    }
    // SAFETY: the out pointer is writable.
    let output = created(|out| unsafe { MFCreateMediaType(out) })?;
    // SAFETY: SetGUID(key, value) on a live media type, then
    // SetCurrentMediaType(stream, reserved, type) on a live reader.
    unsafe {
        let set_guid = output.method::<unsafe extern "system" fn(
            *mut c_void,
            *const Guid,
            *const Guid,
        ) -> Hresult>(SET_GUID);
        set_guid(output.0, &MF_MT_MAJOR_TYPE, &MEDIA_TYPE_VIDEO);
        set_guid(output.0, &MF_MT_SUBTYPE, &VIDEO_FORMAT_NV12);
        let set_type = reader.method::<unsafe extern "system" fn(
            *mut c_void,
            u32,
            *mut u32,
            *mut c_void,
        ) -> Hresult>(SET_CURRENT_MEDIA_TYPE);
        if set_type(reader.0, FIRST_VIDEO_STREAM, null_mut(), output.0) < 0 {
            return None;
        }
    }
    let sample = loop {
        let (mut index, mut flags, mut time, mut sample) = (0u32, 0u32, 0i64, null_mut());
        // SAFETY: ReadSample(stream, control, index, flags, time, sample)
        // reads synchronously into the out pointers.
        let result = unsafe {
            reader.method::<unsafe extern "system" fn(
                *mut c_void,
                u32,
                u32,
                *mut u32,
                *mut u32,
                *mut i64,
                *mut *mut c_void,
            ) -> Hresult>(READ_SAMPLE)(
                reader.0,
                FIRST_VIDEO_STREAM,
                0,
                &mut index,
                &mut flags,
                &mut time,
                &mut sample,
            )
        };
        if !sample.is_null() {
            break Com(sample);
        }
        if result < 0 || flags & (READ_ERROR | END_OF_STREAM) != 0 {
            return None;
        }
    };
    // The type the reader now gives, read after the sample, since the
    // decoder settles the frame size as it decodes.
    // SAFETY: GetCurrentMediaType(stream, type) on a live reader.
    let current = created(|out| unsafe {
        reader.method::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> Hresult>(
            GET_CURRENT_MEDIA_TYPE,
        )(reader.0, FIRST_VIDEO_STREAM, out)
    })?;
    let size = attribute_u64(&current, &MF_MT_FRAME_SIZE)?;
    let (width, height) = ((size >> 32) as u32, size as u32);
    // NV12's stride is the bytes between rows of its luma plane.
    let stride = attribute_u32(&current, &MF_MT_DEFAULT_STRIDE)
        .map(|stride| stride as i32)
        .filter(|stride| *stride > 0)
        .map_or(width as usize, |stride| stride as usize);
    let rotation = attribute_u32(&current, &MF_MT_VIDEO_ROTATION).unwrap_or(0);
    let shown = aperture(&current)
        .filter(|(x, y, w, h)| *w > 0 && *h > 0 && x + w <= width && y + h <= height)
        .unwrap_or((0, 0, width, height));
    let colour = Colour::of(
        attribute_u32(&current, &MF_MT_YUV_MATRIX),
        attribute_u32(&current, &MF_MT_VIDEO_NOMINAL_RANGE),
        attribute_u32(&current, &MF_MT_VIDEO_PRIMARIES),
        shown.3,
    );
    // SAFETY: ConvertToContiguousBuffer(buffer) on a live sample.
    let buffer = created(|out| unsafe {
        sample.method::<unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult>(
            CONVERT_TO_CONTIGUOUS_BUFFER,
        )(sample.0, out)
    })?;
    let (mut data, mut capacity, mut length) = (null_mut::<u8>(), 0u32, 0u32);
    // SAFETY: Lock(data, capacity, length) on a live buffer, unlocked below.
    if unsafe {
        buffer.method::<unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32, *mut u32) -> Hresult>(LOCK)(
            buffer.0,
            &mut data,
            &mut capacity,
            &mut length,
        )
    } < 0
    {
        return None;
    }
    // SAFETY: the locked buffer holds `length` bytes until unlocked.
    let bytes = unsafe { core::slice::from_raw_parts(data, length as usize) };
    let frame =
        nv12(bytes, (width, height), stride, &colour).map(|pixel| upright(pixel, shown, rotation));
    // SAFETY: balances the Lock above.
    unsafe { buffer.method::<unsafe extern "system" fn(*mut c_void) -> Hresult>(UNLOCK)(buffer.0) };
    frame
}

/// How a video's YUV samples become the sRGB pixels Chromium shows.
struct Colour {
    /// The luma weights of red and blue in the YUV matrix.
    red: f32,
    blue: f32,
    /// Whether samples use the full 0-255 range, not 16-235 and 16-240.
    full: bool,
    /// Whether the picture has SMPTE 170M (BT.601) primaries, which
    /// Chromium converts to sRGB's.
    smpte_170m: bool,
}

impl Colour {
    /// The colour of a video from its matrix, nominal range and primaries,
    /// as Media Foundation numbers them. Untagged video is BT.601 with SMPTE
    /// 170M primaries below 720 lines and BT.709 from 720, as Chromium
    /// takes it.
    fn of(
        matrix: Option<u32>,
        range: Option<u32>,
        primaries: Option<u32>,
        shown_height: u32,
    ) -> Colour {
        let high_definition = shown_height >= 720;
        let (red, blue) = match matrix {
            Some(1) => (0.2126, 0.0722),
            Some(2) => (0.299, 0.114),
            Some(3) => (0.212, 0.087),
            Some(4 | 5) => (0.2627, 0.0593),
            _ if high_definition => (0.2126, 0.0722),
            _ => (0.299, 0.114),
        };
        Colour {
            red,
            blue,
            full: range == Some(1),
            // SMPTE 170M, SMPTE 240M and SMPTE C share their primaries.
            smpte_170m: match primaries {
                Some(5 | 6 | 8) => true,
                None | Some(0) => !high_definition,
                _ => false,
            },
        }
    }

    /// The sRGB colour of a YUV sample.
    fn rgb(&self, y: u8, u: u8, v: u8) -> [u8; 3] {
        let (y, u, v) = (f32::from(y), f32::from(u) - 128.0, f32::from(v) - 128.0);
        let (y, u, v) = if self.full {
            (y, u, v)
        } else {
            (
                (y - 16.0) * 255.0 / 219.0,
                u * 255.0 / 224.0,
                v * 255.0 / 224.0,
            )
        };
        let green = 1.0 - self.red - self.blue;
        let rgb = [
            y + 2.0 * (1.0 - self.red) * v,
            y - 2.0 * self.blue * (1.0 - self.blue) / green * u
                - 2.0 * self.red * (1.0 - self.red) / green * v,
            y + 2.0 * (1.0 - self.blue) * u,
        ];
        let rgb = rgb.map(|value| (value + 0.5).clamp(0.0, 255.0) as u8);
        if self.smpte_170m {
            from_smpte_170m(rgb)
        } else {
            rgb
        }
    }
}

/// A colour with SMPTE 170M primaries as sRGB shows it: its linear light,
/// taken with sRGB's curve as Chromium takes video's, mapped to sRGB's
/// primaries.
fn from_smpte_170m(rgb: [u8; 3]) -> [u8; 3] {
    const TO_SRGB: [[f64; 3]; 3] = [
        [0.939_554, 0.050_173, 0.010_272],
        [0.017_775, 0.965_795, 0.016_430],
        [-0.001_622, -0.004_371, 1.005_993],
    ];
    let linear = rgb.map(|channel| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            crate::libm::pow((value + 0.055) / 1.055, 2.4)
        }
    });
    TO_SRGB.map(|row| {
        let value = (row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2]).clamp(0.0, 1.0);
        let encoded = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * crate::libm::pow(value, 1.0 / 2.4) - 0.055
        };
        (encoded * 255.0 + 0.5) as u8
    })
}

/// The pixels of an NV12 picture as sRGB: a luma plane of `height` rows
/// `stride` bytes apart, then a plane of interleaved U and V samples, each
/// pair shared by two by two pixels.
fn nv12<'a>(
    bytes: &'a [u8],
    (width, height): (u32, u32),
    stride: usize,
    colour: &'a Colour,
) -> Option<impl Fn(u32, u32) -> [u8; 4] + 'a> {
    let chroma = stride * height as usize;
    let rows = (height as usize).div_ceil(2);
    if stride < width as usize || bytes.len() < chroma + stride * rows {
        return None;
    }
    Some(move |x: u32, y: u32| {
        let luma = bytes[y as usize * stride + x as usize];
        let at = chroma + (y as usize / 2) * stride + (x as usize / 2) * 2;
        let [red, green, blue] = colour.rgb(luma, bytes[at], bytes[at + 1]);
        [red, green, blue, 255]
    })
}

/// The shown part of a picture, given pixel by pixel, as upright RGBA: the
/// picture is turned clockwise by `rotation` degrees to show it as
/// recorded.
fn upright(
    pixel: impl Fn(u32, u32) -> [u8; 4],
    (left, top, shown_width, shown_height): (u32, u32, u32, u32),
    rotation: u32,
) -> Frame {
    let turned = matches!(rotation, 90 | 270);
    let (out_width, out_height) = if turned {
        (shown_height, shown_width)
    } else {
        (shown_width, shown_height)
    };
    let mut pixels = Vec::with_capacity(out_width as usize * out_height as usize * 4);
    for y in 0..out_height {
        for x in 0..out_width {
            // The source pixel each upright one shows.
            let (source_x, source_y) = match rotation {
                90 => (y, shown_height - 1 - x),
                180 => (shown_width - 1 - x, shown_height - 1 - y),
                270 => (shown_width - 1 - y, x),
                _ => (x, y),
            };
            pixels.extend_from_slice(&pixel(left + source_x, top + source_y));
        }
    }
    Frame {
        width: out_width,
        height: out_height,
        pixels,
    }
}

#[cfg(test)]
mod tests;
