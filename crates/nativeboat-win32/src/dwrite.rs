//! Color emoji and OpenType features with DirectWrite. GDI draws every
//! glyph in one color and applies no OpenType features, so text in an emoji
//! font, and text asking for features such as tabular figures, is laid out
//! and drawn by DirectWrite instead: its color glyph runs give each layer of
//! an emoji its own color, as browsers show them, and its typography applies
//! the features. DirectWrite is loaded when first used; without it, callers
//! keep GDI's text.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::wide;
use core::cell::{RefCell, UnsafeCell};
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{COLORREF, CloseHandle, RECT};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::{CreateThread, Sleep};
use windows_sys::core::{BOOL, GUID, HRESULT, s};

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_FACTORY: GUID = GUID::from_u128(0xb859ee5a_d838_4b5b_a2e8_1adc7d93db48);
const IID_FACTORY2: GUID = GUID::from_u128(0x0439fc60_ca44_4994_8dee_3a9af7b732ec);
const IID_FACTORY5: GUID = GUID::from_u128(0x958db99a_be2a_4f09_af7d_65189803d1d3);
const IID_PIXEL_SNAPPING: GUID = GUID::from_u128(0xeaf3a2da_ecf4_4d24_b644_b34f6842024b);
const IID_TEXT_RENDERER: GUID = GUID::from_u128(0xef8a8135_5cc6_45fe_8825_c5a0724eb819);

const S_OK: HRESULT = 0;
const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as HRESULT;
const DWRITE_FONT_STYLE_ITALIC: u32 = 2;
const DWRITE_FONT_STRETCH_NORMAL: u32 = 5;
const DWRITE_WORD_WRAPPING_NO_WRAP: u32 = 1;

// Vtable slots, after IUnknown's three.
const FACTORY_CREATE_RENDERING_PARAMS: usize = 10;
const FACTORY_CREATE_TEXT_FORMAT: usize = 15;
const FACTORY_CREATE_TYPOGRAPHY: usize = 16;
const FACTORY_GET_GDI_INTEROP: usize = 17;
const FACTORY_CREATE_TEXT_LAYOUT: usize = 18;
const FACTORY2_TRANSLATE_COLOR_GLYPH_RUN: usize = 28;
const FACTORY5_UNPACK_FONT_FILE: usize = 47;
const STREAM_READ_FILE_FRAGMENT: usize = 3;
const STREAM_RELEASE_FILE_FRAGMENT: usize = 4;
const STREAM_GET_FILE_SIZE: usize = 5;
const DWRITE_CONTAINER_TYPE_WOFF: u32 = 1;
const DWRITE_CONTAINER_TYPE_WOFF2: u32 = 2;
const FORMAT_SET_WORD_WRAPPING: usize = 5;
const LAYOUT_SET_TYPOGRAPHY: usize = 40;
const LAYOUT_DRAW: usize = 58;
const TYPOGRAPHY_ADD_FONT_FEATURE: usize = 3;
const LAYOUT_GET_LINE_METRICS: usize = 59;
const LAYOUT_GET_METRICS: usize = 60;
const INTEROP_CREATE_FONT_FACE_FROM_HDC: usize = 6;
const INTEROP_CREATE_BITMAP_RENDER_TARGET: usize = 7;
const FACTORY_CREATE_GLYPH_RUN_ANALYSIS: usize = 23;
const FACTORY_CREATE_FONT_FACE: usize = 9;
const FACTORY_REGISTER_FONT_FILE_LOADER: usize = 13;
const FACTORY5_CREATE_IN_MEMORY_FONT_FILE_LOADER: usize = 44;
const MEMORY_LOADER_CREATE_REFERENCE: usize = 4;
const DWRITE_FONT_SIMULATIONS_BOLD: u32 = 1;
const DWRITE_FONT_SIMULATIONS_OBLIQUE: u32 = 2;
const ANALYSIS_GET_ALPHA_TEXTURE_BOUNDS: usize = 3;
const ANALYSIS_CREATE_ALPHA_TEXTURE: usize = 4;
const DWRITE_TEXTURE_CLEARTYPE_3X1: u32 = 1;
const DWRITE_RENDERING_MODE_NATURAL: u32 = 4;
const DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC: u32 = 5;
const DWRITE_MEASURING_MODE_NATURAL: u32 = 0;
const TARGET_DRAW_GLYPH_RUN: usize = 3;
const TARGET_GET_MEMORY_DC: usize = 4;
const TARGET_SET_PIXELS_PER_DIP: usize = 6;
const TARGET_RESIZE: usize = 10;
const ENUMERATOR_MOVE_NEXT: usize = 3;
const ENUMERATOR_GET_CURRENT_RUN: usize = 4;

/// The method in a slot of a COM object's vtable.
///
/// # Safety
/// The object is live and the slot holds a method of type `F`.
unsafe fn method<F: Copy>(object: *mut c_void, slot: usize) -> F {
    // SAFETY: a COM object starts with its vtable, an array of methods.
    unsafe {
        let vtable = *object.cast::<*const usize>();
        core::mem::transmute_copy::<usize, F>(&*vtable.add(slot))
    }
}

/// An owned reference to a COM object.
struct Com(*mut c_void);

impl Com {
    /// Calls a method that returns a new object through its last argument.
    ///
    /// # Safety
    /// The slot holds a method taking only the out pointer.
    unsafe fn create(&self, slot: usize) -> Option<Com> {
        let mut out = null_mut();
        // SAFETY: as the caller promises.
        let result = unsafe {
            method::<unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT>(
                self.0, slot,
            )(self.0, &mut out)
        };
        (result >= 0 && !out.is_null()).then_some(Com(out))
    }

    /// The object's interface with an ID, if it has one.
    fn query(&self, iid: &GUID) -> Option<Com> {
        let mut out = null_mut();
        // SAFETY: slot 0 of every COM object is QueryInterface.
        let result = unsafe {
            method::<unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT>(
                self.0, 0,
            )(self.0, iid, &mut out)
        };
        (result >= 0 && !out.is_null()).then_some(Com(out))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: slot 2 of every COM object is Release.
        unsafe { method::<unsafe extern "system" fn(*mut c_void) -> u32>(self.0, 2)(self.0) };
    }
}

#[repr(C)]
#[derive(Default)]
struct TextMetrics {
    left: f32,
    top: f32,
    width: f32,
    width_including_trailing_whitespace: f32,
    height: f32,
    layout_width: f32,
    layout_height: f32,
    max_bidi_reordering_depth: u32,
    line_count: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct LineMetrics {
    length: u32,
    trailing_whitespace_length: u32,
    newline_length: u32,
    height: f32,
    baseline: f32,
    is_trimmed: BOOL,
}

#[repr(C)]
struct GlyphRun {
    font_face: *mut c_void,
    font_em_size: f32,
    glyph_count: u32,
    glyph_indices: *const u16,
    glyph_advances: *const f32,
    glyph_offsets: *const c_void,
    is_sideways: BOOL,
    bidi_level: u32,
}

#[repr(C)]
struct ColorGlyphRun {
    glyph_run: GlyphRun,
    glyph_run_description: *mut c_void,
    baseline_origin_x: f32,
    baseline_origin_y: f32,
    run_color: [f32; 4],
    palette_index: u16,
}

/// What a single-line DirectWrite layout measures, in pixels.
struct Measured {
    layout: Com,
    width: f32,
    height: f32,
    baseline: f32,
}

/// DirectWrite's factory and what drawing reuses.
struct Writer {
    factory: Com,
    /// `IDWriteFactory2`, which splits color glyphs into layers (Windows 8.1
    /// and later).
    color: Option<Com>,
    interop: Com,
    params: Com,
    formats: HashMap<(String, u16, bool, u32), Com>,
    /// Font faces of GDI fonts, by face name, weight and slant: making one
    /// costs DirectWrite milliseconds, and every size of a font shares it.
    faces: HashMap<FaceKey, Com>,
    /// A bitmap render target and its size, grown as text needs.
    target: Option<(Com, i32, i32)>,
}

local! {
    static WRITER: RefCell<Option<Option<Writer>>> = RefCell::new(None);
}

/// DirectWrite's shared factory, which every thread of the process shares
/// with its cached font data; `None` without DirectWrite.
fn shared_factory() -> Option<Com> {
    let name = wide("dwrite.dll");
    // SAFETY: loads a system library by name.
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    // SAFETY: looks up an export by its NUL-terminated name.
    let create = unsafe { GetProcAddress(module, s!("DWriteCreateFactory")) }?;
    // SAFETY: the export has the documented DWriteCreateFactory signature.
    let create = unsafe {
        core::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(u32, *const GUID, *mut *mut c_void) -> HRESULT,
        >(create)
    };
    let mut factory = null_mut();
    // SAFETY: a shared factory is written to `factory`.
    let result = unsafe { create(0, &IID_FACTORY, &mut factory) };
    (result >= 0 && !factory.is_null()).then(|| Com(factory))
}

/// A face as GDI selects it: its family name as a `LOGFONTW` holds it, its
/// weight and its slant.
type FaceKey = (Vec<u16>, i32, u8);

/// Faces worker threads make ahead of painting, by the face GDI selects,
/// for the UI thread to take: `0` while one is being made.
struct Ready {
    busy: AtomicBool,
    faces: UnsafeCell<Vec<(FaceKey, usize)>>,
}

// SAFETY: `busy` guards `faces`, and the faces are DirectWrite objects,
// which any thread may use.
unsafe impl Sync for Ready {}

impl Ready {
    /// Works with the faces made, alone.
    fn with<R>(&self, work: impl FnOnce(&mut Vec<(FaceKey, usize)>) -> R) -> R {
        // The faces are held for moments: waiting spins.
        while self.busy.swap(true, Ordering::Acquire) {}
        // SAFETY: holding `busy` gives this thread the only access.
        let result = work(unsafe { &mut *self.faces.get() });
        self.busy.store(false, Ordering::Release);
        result
    }

    /// The face made for a GDI face, taken from the faces made, once it is
    /// made; `None` when none is made or being made.
    fn take(&self, key: &FaceKey) -> Option<Com> {
        loop {
            let found = self.with(|faces| {
                let index = faces.iter().position(|(made, _)| made == key)?;
                if faces[index].1 == 0 {
                    return Some(None);
                }
                Some(Some(Com(faces.swap_remove(index).1 as *mut c_void)))
            })?;
            match found {
                Some(face) => return Some(face),
                // Waiting costs less than reading the font again.
                // SAFETY: only waits.
                None => unsafe { Sleep(1) },
            }
        }
    }
}

static READY: Ready = Ready {
    busy: AtomicBool::new(false),
    faces: UnsafeCell::new(Vec::new()),
};

/// Starts making, on a worker thread, the DirectWrite face of the font a
/// GDI font description selects, unless one is made or being made. Text
/// is measured before it is painted, and DirectWrite takes milliseconds
/// to read a font, which painting would otherwise wait for. `memory` says
/// the font is one this process loaded from memory, which is read from a
/// copy of its data.
pub(crate) fn prepare(logfont: &LOGFONTW, memory: bool) {
    let key = (
        logfont.lfFaceName.to_vec(),
        logfont.lfWeight,
        logfont.lfItalic,
    );
    let new = READY.with(|faces| {
        let new = faces.iter().all(|(made, _)| *made != key);
        if new {
            faces.push((key.clone(), 0));
        }
        new
    });
    if !new {
        return;
    }
    let job = Box::into_raw(Box::new((*logfont, memory)));
    // SAFETY: the thread owns the job and frees it; without a thread, this
    // one does the work, and closing no thread does nothing.
    unsafe {
        let thread = CreateThread(
            null(),
            0,
            Some(prepared),
            job.cast_const().cast(),
            0,
            null_mut(),
        );
        let _ = thread.is_null() && prepared(job.cast()) == 0;
        CloseHandle(thread);
    }
}

/// A worker thread's work: makes the face of a font, and hands it over.
unsafe extern "system" fn prepared(job: *mut c_void) -> u32 {
    // SAFETY: `prepare` hands this thread its own job.
    let (logfont, memory) = *unsafe { Box::from_raw(job.cast::<(LOGFONTW, bool)>()) };
    let key = (
        logfont.lfFaceName.to_vec(),
        logfont.lfWeight,
        logfont.lfItalic,
    );
    let face = shared_factory().and_then(|factory| {
        // SAFETY: a memory DC of this thread selects the font; both are
        // deleted here.
        unsafe {
            let font = CreateFontIndirectW(&logfont);
            let dc = CreateCompatibleDC(null_mut());
            SelectObject(dc, font);
            let copy = memory
                .then(|| memory_loader(&factory))
                .flatten()
                .and_then(|loader| memory_copy(&factory, loader.0, dc, &logfont));
            let face = copy.or_else(|| {
                // SAFETY: the slot takes only the out pointer.
                let interop = factory.create(FACTORY_GET_GDI_INTEROP)?;
                gdi_face(&interop, dc)
            });
            DeleteDC(dc);
            DeleteObject(font);
            face
        }
    });
    READY.with(|faces| match face {
        Some(face) => {
            for (made, slot) in faces.iter_mut() {
                if *made == key {
                    *slot = face.0 as usize;
                }
            }
            core::mem::forget(face);
        }
        None => faces.retain(|(made, _)| *made != key),
    });
    0
}

/// The face GDI interop makes for the font selected into a DC.
fn gdi_face(interop: &Com, dc: HDC) -> Option<Com> {
    let mut face = null_mut();
    // SAFETY: the DC is live; the face is written to `face`.
    let result = unsafe {
        method::<unsafe extern "system" fn(*mut c_void, HDC, *mut *mut c_void) -> HRESULT>(
            interop.0,
            INTEROP_CREATE_FONT_FACE_FROM_HDC,
        )(interop.0, dc, &mut face)
    };
    (result >= 0 && !face.is_null()).then(|| Com(face))
}

/// The face of the font selected into a DC, which this process loaded from
/// memory, read from a copy of the font's data. GDI interop reads such
/// fonts, web fonts among them, through GDI, which makes making their faces
/// and rasterizing their glyphs several times slower than from a file. The
/// face is emboldened where 600 or heavier is asked of a lighter font, and
/// slanted where italic is asked of an upright one, as Blink synthesizes
/// them. `None` without an in-memory loader.
fn memory_copy(factory: &Com, loader: *mut c_void, dc: HDC, logfont: &LOGFONTW) -> Option<Com> {
    // SAFETY: the DC is live; a null buffer asks for the font's size.
    let size = unsafe { GetFontData(dc, 0, 0, null_mut(), 0) };
    let mut data = alloc::vec![0u8; size as usize];
    // SAFETY: the buffer holds `size` bytes.
    let read = unsafe { GetFontData(dc, 0, 0, data.as_mut_ptr().cast(), size) };
    (size != 0 && size != GDI_ERROR as u32 && read == size).then_some(())?;
    // Fonts of CFF outlines are face type 0, TrueType ones 1.
    let kind = u32::from(!data.starts_with(b"OTTO"));
    let (weight, italic) = crate::fonts::own_style(&data).unwrap_or((400, false));
    let bold = logfont.lfWeight >= 600 && weight < 600;
    let slanted = logfont.lfItalic != 0 && !italic;
    let simulations = (u32::from(bold) * DWRITE_FONT_SIMULATIONS_BOLD)
        | (u32::from(slanted) * DWRITE_FONT_SIMULATIONS_OBLIQUE);
    face_from_memory(factory, loader, &data, kind, simulations)
}

/// An in-memory font file loader, registered with a factory.
fn memory_loader(factory: &Com) -> Option<Com> {
    let fifth = factory.query(&IID_FACTORY5)?;
    // SAFETY: the slot takes only the out pointer.
    let loader = unsafe { fifth.create(FACTORY5_CREATE_IN_MEMORY_FONT_FILE_LOADER) }?;
    // SAFETY: the factory and loader are live.
    let result = unsafe {
        method::<unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT>(
            factory.0,
            FACTORY_REGISTER_FONT_FILE_LOADER,
        )(factory.0, loader.0)
    };
    (result >= 0).then_some(loader)
}

/// A face of a font, of a kind and with simulations, read from a copy of
/// its data that an in-memory loader of the factory keeps.
fn face_from_memory(
    factory: &Com,
    loader: *mut c_void,
    data: &[u8],
    kind: u32,
    simulations: u32,
) -> Option<Com> {
    let mut copy = null_mut();
    // SAFETY: the loader copies the data, which a null owner asks for; the
    // file is written to `copy`.
    let result = unsafe {
        method::<
            unsafe extern "system" fn(
                *mut c_void,
                *mut c_void,
                *const c_void,
                u32,
                *mut c_void,
                *mut *mut c_void,
            ) -> HRESULT,
        >(loader, MEMORY_LOADER_CREATE_REFERENCE)(
            loader,
            factory.0,
            data.as_ptr().cast(),
            u32::try_from(data.len()).ok()?,
            null_mut(),
            &mut copy,
        )
    };
    (result >= 0 && !copy.is_null()).then_some(())?;
    let copy = Com(copy);
    let mut made = null_mut();
    // SAFETY: one live file of one font; the face is written to `made`.
    let result = unsafe {
        method::<
            unsafe extern "system" fn(
                *mut c_void,
                u32,
                u32,
                *const *mut c_void,
                u32,
                u32,
                *mut *mut c_void,
            ) -> HRESULT,
        >(factory.0, FACTORY_CREATE_FONT_FACE)(
            factory.0,
            kind,
            1,
            &copy.0,
            0,
            simulations,
            &mut made,
        )
    };
    (result >= 0 && !made.is_null()).then(|| Com(made))
}

impl Writer {
    fn new() -> Option<Writer> {
        let factory = shared_factory()?;
        // SAFETY: the slots take only the out pointer.
        let (interop, params) = unsafe {
            (
                factory.create(FACTORY_GET_GDI_INTEROP)?,
                factory.create(FACTORY_CREATE_RENDERING_PARAMS)?,
            )
        };
        Some(Writer {
            color: factory.query(&IID_FACTORY2),
            factory,
            interop,
            params,
            formats: HashMap::new(),
            faces: HashMap::new(),
            target: None,
        })
    }

    /// A single-line layout of text in a font, sized in pixels, with
    /// OpenType features as packed tags and values.
    fn measure(
        &mut self,
        text: &[u16],
        family: &str,
        size: f32,
        weight: u16,
        italic: bool,
        features: &[(u32, u32)],
    ) -> Option<Measured> {
        let key = (family.to_owned(), weight, italic, size.to_bits());
        if !self.formats.contains_key(&key) {
            let name = wide(family);
            let locale = wide("");
            let mut format = null_mut();
            // SAFETY: the strings are NUL-terminated; a null collection is
            // the system's.
            let result = unsafe {
                method::<
                    unsafe extern "system" fn(
                        *mut c_void,
                        *const u16,
                        *mut c_void,
                        u32,
                        u32,
                        u32,
                        f32,
                        *const u16,
                        *mut *mut c_void,
                    ) -> HRESULT,
                >(self.factory.0, FACTORY_CREATE_TEXT_FORMAT)(
                    self.factory.0,
                    name.as_ptr(),
                    null_mut(),
                    u32::from(weight.clamp(1, 999)),
                    if italic { DWRITE_FONT_STYLE_ITALIC } else { 0 },
                    DWRITE_FONT_STRETCH_NORMAL,
                    size.max(1.0),
                    locale.as_ptr(),
                    &mut format,
                )
            };
            if result < 0 || format.is_null() {
                return None;
            }
            // SAFETY: the format is live.
            unsafe {
                method::<unsafe extern "system" fn(*mut c_void, u32) -> HRESULT>(
                    format,
                    FORMAT_SET_WORD_WRAPPING,
                )(format, DWRITE_WORD_WRAPPING_NO_WRAP);
            }
            // Pages use few emoji fonts and sizes; a full cache starts over.
            if self.formats.len() >= 64 {
                self.formats.clear();
            }
            self.formats.insert(key.clone(), Com(format));
        }
        let format = self.formats.get(&key)?.0;
        let mut layout = null_mut();
        // SAFETY: the text and its length match; the format is live.
        let result = unsafe {
            method::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *const u16,
                    u32,
                    *mut c_void,
                    f32,
                    f32,
                    *mut *mut c_void,
                ) -> HRESULT,
            >(self.factory.0, FACTORY_CREATE_TEXT_LAYOUT)(
                self.factory.0,
                text.as_ptr(),
                text.len() as u32,
                format,
                f32::MAX,
                f32::MAX,
                &mut layout,
            )
        };
        if result < 0 || layout.is_null() {
            return None;
        }
        let layout = Com(layout);
        if !features.is_empty() {
            self.typography(&layout, text.len() as u32, features)?;
        }
        let mut metrics = TextMetrics::default();
        let mut line = LineMetrics::default();
        let mut lines = 0;
        // SAFETY: the layout is live; the outputs are writable and one line
        // fits.
        unsafe {
            method::<unsafe extern "system" fn(*mut c_void, *mut TextMetrics) -> HRESULT>(
                layout.0,
                LAYOUT_GET_METRICS,
            )(layout.0, &mut metrics);
            method::<
                unsafe extern "system" fn(*mut c_void, *mut LineMetrics, u32, *mut u32) -> HRESULT,
            >(layout.0, LAYOUT_GET_LINE_METRICS)(layout.0, &mut line, 1, &mut lines);
        }
        Some(Measured {
            layout,
            width: metrics.width_including_trailing_whitespace,
            height: metrics.height,
            baseline: line.baseline,
        })
    }

    /// Applies OpenType features to the whole of a layout's text.
    fn typography(&self, layout: &Com, length: u32, features: &[(u32, u32)]) -> Option<()> {
        #[repr(C)]
        struct Feature {
            tag: u32,
            parameter: u32,
        }
        #[repr(C)]
        struct Range {
            start: u32,
            length: u32,
        }
        // SAFETY: the slot takes only the out pointer.
        let typography = unsafe { self.factory.create(FACTORY_CREATE_TYPOGRAPHY)? };
        // SAFETY: the typography and the layout are live; the structures
        // are passed by value as DirectWrite declares them.
        unsafe {
            for (tag, parameter) in features {
                method::<unsafe extern "system" fn(*mut c_void, Feature) -> HRESULT>(
                    typography.0,
                    TYPOGRAPHY_ADD_FONT_FEATURE,
                )(
                    typography.0,
                    Feature {
                        tag: *tag,
                        parameter: *parameter,
                    },
                );
            }
            let result = method::<
                unsafe extern "system" fn(*mut c_void, *mut c_void, Range) -> HRESULT,
            >(layout.0, LAYOUT_SET_TYPOGRAPHY)(
                layout.0, typography.0, Range { start: 0, length }
            );
            (result >= 0).then_some(())
        }
    }

    /// The bitmap render target, at least of a size.
    fn target(&mut self, dc: HDC, width: i32, height: i32) -> Option<*mut c_void> {
        match &mut self.target {
            Some((target, kept_width, kept_height)) => {
                if width > *kept_width || height > *kept_height {
                    *kept_width = width.max(*kept_width);
                    *kept_height = height.max(*kept_height);
                    // SAFETY: the target is live.
                    let result = unsafe {
                        method::<unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT>(
                            target.0,
                            TARGET_RESIZE,
                        )(target.0, *kept_width as u32, *kept_height as u32)
                    };
                    if result < 0 {
                        self.target = None;
                        return None;
                    }
                }
            }
            None => {
                let mut target = null_mut();
                // SAFETY: the DC is live; the target is written to `target`.
                let result =
                    unsafe {
                        method::<
                            unsafe extern "system" fn(
                                *mut c_void,
                                HDC,
                                u32,
                                u32,
                                *mut *mut c_void,
                            ) -> HRESULT,
                        >(
                            self.interop.0, INTEROP_CREATE_BITMAP_RENDER_TARGET
                        )(
                            self.interop.0, dc, width as u32, height as u32, &mut target
                        )
                    };
                if result < 0 || target.is_null() {
                    return None;
                }
                // Sizes are already in pixels, whatever the DC's DPI.
                // SAFETY: the target is live.
                unsafe {
                    method::<unsafe extern "system" fn(*mut c_void, f32) -> HRESULT>(
                        target,
                        TARGET_SET_PIXELS_PER_DIP,
                    )(target, 1.0);
                }
                self.target = Some((Com(target), width, height));
            }
        }
        self.target.as_ref().map(|(target, _, _)| target.0)
    }
}

/// Runs a function with DirectWrite, loading it the first time.
fn with_writer<R>(run: impl FnOnce(&mut Writer) -> Option<R>) -> Option<R> {
    WRITER.with_borrow_mut(|writer| {
        writer
            .get_or_insert_with(Writer::new)
            .as_mut()
            .and_then(run)
    })
}

/// The TrueType or OpenType font a WOFF or WOFF2 file packs, as
/// DirectWrite unpacks it (Windows 10 1703 and later); `None` for other
/// data or without that DirectWrite.
pub fn unpack_font(data: &[u8]) -> Option<Vec<u8>> {
    let container = match data.get(..4)? {
        b"wOF2" => DWRITE_CONTAINER_TYPE_WOFF2,
        b"wOFF" => DWRITE_CONTAINER_TYPE_WOFF,
        _ => return None,
    };
    let size = u32::try_from(data.len()).ok()?;
    with_writer(|writer| {
        let factory = writer.factory.query(&IID_FACTORY5)?;
        let mut stream = null_mut();
        // SAFETY: the data and its size match; the stream is written to
        // `stream`.
        let result = unsafe {
            method::<
                unsafe extern "system" fn(
                    *mut c_void,
                    u32,
                    *const c_void,
                    u32,
                    *mut *mut c_void,
                ) -> HRESULT,
            >(factory.0, FACTORY5_UNPACK_FONT_FILE)(
                factory.0,
                container,
                data.as_ptr().cast(),
                size,
                &mut stream,
            )
        };
        if result < 0 || stream.is_null() {
            return None;
        }
        let stream = Com(stream);
        let mut length = 0u64;
        // SAFETY: the stream is live; the output is writable.
        let result = unsafe {
            method::<unsafe extern "system" fn(*mut c_void, *mut u64) -> HRESULT>(
                stream.0,
                STREAM_GET_FILE_SIZE,
            )(stream.0, &mut length)
        };
        let length = usize::try_from(length).ok().filter(|_| result >= 0)?;
        let mut start: *const c_void = null();
        let mut context = null_mut();
        // SAFETY: the whole file is one fragment, read and released here.
        unsafe {
            let result = method::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *mut *const c_void,
                    u64,
                    u64,
                    *mut *mut c_void,
                ) -> HRESULT,
            >(stream.0, STREAM_READ_FILE_FRAGMENT)(
                stream.0,
                &mut start,
                0,
                length as u64,
                &mut context,
            );
            if result < 0 || start.is_null() {
                return None;
            }
            let font = core::slice::from_raw_parts(start.cast::<u8>(), length).to_vec();
            method::<unsafe extern "system" fn(*mut c_void, *mut c_void)>(
                stream.0,
                STREAM_RELEASE_FILE_FRAGMENT,
            )(stream.0, context);
            Some(font)
        }
    })
}

/// The width in pixels of text in a font, as DirectWrite lays it out; `None`
/// without DirectWrite.
pub(crate) fn width(
    text: &[u16],
    family: &str,
    size: f32,
    weight: u16,
    italic: bool,
    features: &[(u32, u32)],
) -> Option<f32> {
    with_writer(|writer| {
        Some(
            writer
                .measure(text, family, size, weight, italic, features)?
                .width,
        )
    })
}

/// Draws text in a font with its color glyphs, starting at a point on the
/// baseline in the DC's coordinates, and returns its width in pixels. Text
/// in other colors takes the color given. `None` leaves the DC untouched:
/// without DirectWrite, or under a transform that scales or turns text.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    dc: HDC,
    x: i32,
    baseline: i32,
    text: &[u16],
    family: &str,
    size: f32,
    weight: u16,
    italic: bool,
    features: &[(u32, u32)],
    color: COLORREF,
) -> Option<f32> {
    let mut transform = XFORM::default();
    // SAFETY: the DC is live and `transform` writable.
    unsafe { GetWorldTransform(dc, &mut transform) };
    if transform.eM11 != 1.0
        || transform.eM22 != 1.0
        || transform.eM12 != 0.0
        || transform.eM21 != 0.0
    {
        return None;
    }
    with_writer(|writer| {
        let measured = writer.measure(text, family, size, weight, italic, features)?;
        // Glyphs may reach past their advances, so the area has a margin.
        let margin = (size / 2.0).ceil() as i32;
        let left = x + transform.eDx.round() as i32 - margin;
        let top =
            baseline + transform.eDy.round() as i32 - measured.baseline.ceil() as i32 - margin;
        let width = measured.width.ceil() as i32 + 2 * margin;
        let height = measured.height.ceil() as i32 + 2 * margin;
        let target = writer.target(dc, width, height)?;
        let mut renderer = Renderer {
            vtable: &RENDERER_VTABLE,
            target,
            color: writer.color.as_ref().map_or(null_mut(), |color| color.0),
            params: writer.params.0,
            text_color: color,
        };
        // SAFETY: the DCs, the target, the layout and the renderer are live
        // for the calls; the world transform is restored after.
        unsafe {
            let memory = method::<unsafe extern "system" fn(*mut c_void) -> HDC>(
                target,
                TARGET_GET_MEMORY_DC,
            )(target);
            ModifyWorldTransform(dc, null(), MWT_IDENTITY);
            // DirectWrite draws over what is already painted there.
            BitBlt(memory, 0, 0, width, height, dc, left, top, SRCCOPY);
            method::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *const c_void,
                    *mut c_void,
                    f32,
                    f32,
                ) -> HRESULT,
            >(measured.layout.0, LAYOUT_DRAW)(
                measured.layout.0,
                null(),
                (&raw mut renderer).cast(),
                margin as f32,
                margin as f32 + (measured.baseline.ceil() - measured.baseline),
            );
            BitBlt(dc, left, top, width, height, memory, 0, 0, SRCCOPY);
            SetWorldTransform(dc, &transform);
        }
        Some(measured.width)
    })
}

/// What DirectWrite's ClearType rasterizer covers of glyphs: the device
/// pixels it reaches and, for each, the coverage of its red, green and blue
/// thirds, row by row.
pub(crate) struct Coverage {
    pub bounds: RECT,
    pub thirds: Vec<u8>,
}

/// The ClearType coverage of glyphs of the font selected into a DC, at a
/// size in pixels, placed by their advances from a point on the baseline in
/// the DC's device pixels, as Skia rasterizes them with DirectWrite before
/// it turns them to grayscale. `symmetric` smooths vertically as well as
/// horizontally. `None` without DirectWrite or for a font it cannot read.
pub(crate) fn coverage(
    dc: HDC,
    glyphs: &[u16],
    advances: &[f32],
    size: f32,
    origin: (f32, f32),
    symmetric: bool,
) -> Option<Coverage> {
    let mut logfont = LOGFONTW::default();
    // SAFETY: reads the selected font's description into a LOGFONTW.
    unsafe {
        let font = GetCurrentObject(dc, OBJ_FONT as u32);
        GetObjectW(
            font,
            size_of::<LOGFONTW>() as i32,
            (&raw mut logfont).cast(),
        );
    }
    let key = (
        logfont.lfFaceName.to_vec(),
        logfont.lfWeight,
        logfont.lfItalic,
    );
    with_writer(|writer| {
        if !writer.faces.contains_key(&key) {
            let face = match READY.take(&key) {
                Some(face) => face,
                // A font no worker read, which GDI interop reads.
                None => gdi_face(&writer.interop, dc)?,
            };
            writer.faces.insert(key.clone(), face);
        }
        let face = writer.faces.get(&key)?;
        let run = GlyphRun {
            font_face: face.0,
            font_em_size: size,
            glyph_count: glyphs.len() as u32,
            glyph_indices: glyphs.as_ptr(),
            glyph_advances: advances.as_ptr(),
            glyph_offsets: null(),
            is_sideways: 0,
            bidi_level: 0,
        };
        let mode = if symmetric {
            DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC
        } else {
            DWRITE_RENDERING_MODE_NATURAL
        };
        let mut analysis = null_mut();
        // SAFETY: the run's arrays hold `glyph_count` entries and outlive
        // the call; the analysis is written to `analysis`.
        let result = unsafe {
            method::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *const GlyphRun,
                    f32,
                    *const c_void,
                    u32,
                    u32,
                    f32,
                    f32,
                    *mut *mut c_void,
                ) -> HRESULT,
            >(writer.factory.0, FACTORY_CREATE_GLYPH_RUN_ANALYSIS)(
                writer.factory.0,
                &run,
                1.0,
                null(),
                mode,
                DWRITE_MEASURING_MODE_NATURAL,
                origin.0,
                origin.1,
                &mut analysis,
            )
        };
        (result >= 0 && !analysis.is_null()).then_some(())?;
        let analysis = Com(analysis);
        let mut bounds = RECT::default();
        // SAFETY: the analysis is live; `bounds` is writable.
        let result = unsafe {
            method::<unsafe extern "system" fn(*mut c_void, u32, *mut RECT) -> HRESULT>(
                analysis.0,
                ANALYSIS_GET_ALPHA_TEXTURE_BOUNDS,
            )(analysis.0, DWRITE_TEXTURE_CLEARTYPE_3X1, &mut bounds)
        };
        (result >= 0).then_some(())?;
        let width = usize::try_from(bounds.right - bounds.left).unwrap_or(0);
        let height = usize::try_from(bounds.bottom - bounds.top).unwrap_or(0);
        let mut thirds = alloc::vec![0u8; width * height * 3];
        if !thirds.is_empty() {
            // SAFETY: the buffer holds three bytes for each pixel of the
            // bounds, as the call requires.
            let result = unsafe {
                method::<
                    unsafe extern "system" fn(
                        *mut c_void,
                        u32,
                        *const RECT,
                        *mut u8,
                        u32,
                    ) -> HRESULT,
                >(analysis.0, ANALYSIS_CREATE_ALPHA_TEXTURE)(
                    analysis.0,
                    DWRITE_TEXTURE_CLEARTYPE_3X1,
                    &bounds,
                    thirds.as_mut_ptr(),
                    thirds.len() as u32,
                )
            };
            (result >= 0).then_some(())?;
        }
        Some(Coverage { bounds, thirds })
    })
}

/// An `IDWriteTextRenderer` that draws glyph runs, layer by layer for color
/// glyphs, into a bitmap render target.
#[repr(C)]
struct Renderer {
    vtable: *const RendererVtable,
    target: *mut c_void,
    /// `IDWriteFactory2`, or null.
    color: *mut c_void,
    params: *mut c_void,
    text_color: COLORREF,
}

#[repr(C)]
struct RendererVtable {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    is_pixel_snapping_disabled:
        unsafe extern "system" fn(*mut c_void, *const c_void, *mut BOOL) -> HRESULT,
    get_current_transform:
        unsafe extern "system" fn(*mut c_void, *const c_void, *mut [f32; 6]) -> HRESULT,
    get_pixels_per_dip: unsafe extern "system" fn(*mut c_void, *const c_void, *mut f32) -> HRESULT,
    draw_glyph_run: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        f32,
        f32,
        u32,
        *const GlyphRun,
        *const c_void,
        *mut c_void,
    ) -> HRESULT,
    draw_line: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        f32,
        f32,
        *const c_void,
        *mut c_void,
    ) -> HRESULT,
    draw_strikethrough: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        f32,
        f32,
        *const c_void,
        *mut c_void,
    ) -> HRESULT,
    draw_inline_object: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        f32,
        f32,
        *mut c_void,
        BOOL,
        BOOL,
        *mut c_void,
    ) -> HRESULT,
}

static RENDERER_VTABLE: RendererVtable = RendererVtable {
    query_interface: renderer_query_interface,
    add_ref: renderer_reference,
    release: renderer_reference,
    is_pixel_snapping_disabled: renderer_snapping,
    get_current_transform: renderer_transform,
    get_pixels_per_dip: renderer_pixels_per_dip,
    draw_glyph_run: renderer_draw_glyph_run,
    draw_line: renderer_draw_line,
    draw_strikethrough: renderer_draw_line,
    draw_inline_object: renderer_draw_inline_object,
};

unsafe extern "system" fn renderer_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: DirectWrite passes a live IID and a writable out pointer.
    unsafe {
        let iid = &*iid;
        let known = [IID_IUNKNOWN, IID_PIXEL_SNAPPING, IID_TEXT_RENDERER];
        if known.iter().any(|candidate| {
            candidate.data1 == iid.data1
                && candidate.data2 == iid.data2
                && candidate.data3 == iid.data3
                && candidate.data4 == iid.data4
        }) {
            *out = this;
            S_OK
        } else {
            *out = null_mut();
            E_NOINTERFACE
        }
    }
}

/// The renderer lives on the stack for one draw, so references are not
/// counted.
unsafe extern "system" fn renderer_reference(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn renderer_snapping(
    _: *mut c_void,
    _: *const c_void,
    disabled: *mut BOOL,
) -> HRESULT {
    // SAFETY: DirectWrite passes a writable pointer.
    unsafe { *disabled = 0 };
    S_OK
}

unsafe extern "system" fn renderer_transform(
    _: *mut c_void,
    _: *const c_void,
    matrix: *mut [f32; 6],
) -> HRESULT {
    // SAFETY: DirectWrite passes a writable matrix.
    unsafe { *matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };
    S_OK
}

unsafe extern "system" fn renderer_pixels_per_dip(
    _: *mut c_void,
    _: *const c_void,
    pixels: *mut f32,
) -> HRESULT {
    // SAFETY: DirectWrite passes a writable pointer.
    unsafe { *pixels = 1.0 };
    S_OK
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn renderer_draw_glyph_run(
    this: *mut c_void,
    _: *const c_void,
    x: f32,
    y: f32,
    measuring_mode: u32,
    run: *const GlyphRun,
    description: *const c_void,
    _: *mut c_void,
) -> HRESULT {
    // SAFETY: `this` is the renderer `draw` passed, whose target and
    // factory are live; DirectWrite passes a live glyph run.
    unsafe {
        let renderer = &*this.cast::<Renderer>();
        let draw = |x: f32, y: f32, run: *const GlyphRun, color: COLORREF| {
            method::<
                unsafe extern "system" fn(
                    *mut c_void,
                    f32,
                    f32,
                    u32,
                    *const GlyphRun,
                    *mut c_void,
                    COLORREF,
                    *mut RECT,
                ) -> HRESULT,
            >(renderer.target, TARGET_DRAW_GLYPH_RUN)(
                renderer.target,
                x,
                y,
                measuring_mode,
                run,
                renderer.params,
                color,
                null_mut(),
            )
        };
        let mut layers = null_mut();
        let translated = !renderer.color.is_null()
            && method::<
                unsafe extern "system" fn(
                    *mut c_void,
                    f32,
                    f32,
                    *const GlyphRun,
                    *const c_void,
                    u32,
                    *const [f32; 6],
                    u32,
                    *mut *mut c_void,
                ) -> HRESULT,
            >(renderer.color, FACTORY2_TRANSLATE_COLOR_GLYPH_RUN)(
                renderer.color,
                x,
                y,
                run,
                description,
                measuring_mode,
                null(),
                0,
                &mut layers,
            ) >= 0
            && !layers.is_null();
        // Glyphs without color layers take the text's color.
        if !translated {
            return draw(x, y, run, renderer.text_color);
        }
        let layers = Com(layers);
        loop {
            let mut more: BOOL = 0;
            let moved = method::<unsafe extern "system" fn(*mut c_void, *mut BOOL) -> HRESULT>(
                layers.0,
                ENUMERATOR_MOVE_NEXT,
            )(layers.0, &mut more);
            if moved < 0 || more == 0 {
                break;
            }
            let mut layer: *const ColorGlyphRun = null();
            let current = method::<
                unsafe extern "system" fn(*mut c_void, *mut *const ColorGlyphRun) -> HRESULT,
            >(layers.0, ENUMERATOR_GET_CURRENT_RUN)(layers.0, &mut layer);
            if current < 0 || layer.is_null() {
                break;
            }
            let layer = &*layer;
            // Palette index 0xFFFF is the text's color.
            let color = if layer.palette_index == 0xFFFF {
                renderer.text_color
            } else {
                let [r, g, b, _] = layer
                    .run_color
                    .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u32);
                r | (g << 8) | (b << 16)
            };
            draw(
                layer.baseline_origin_x,
                layer.baseline_origin_y,
                &layer.glyph_run,
                color,
            );
        }
        S_OK
    }
}

/// Lines are drawn with the rest of the text's decorations.
unsafe extern "system" fn renderer_draw_line(
    _: *mut c_void,
    _: *const c_void,
    _: f32,
    _: f32,
    _: *const c_void,
    _: *mut c_void,
) -> HRESULT {
    S_OK
}

/// Plain text has no inline objects.
unsafe extern "system" fn renderer_draw_inline_object(
    _: *mut c_void,
    _: *const c_void,
    _: f32,
    _: f32,
    _: *mut c_void,
    _: BOOL,
    _: BOOL,
    _: *mut c_void,
) -> HRESULT {
    S_OK
}

#[cfg(test)]
mod tests;
