use super::*;

/// A 32-bit DIB in a memory DC, filled white, and its pixels.
struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    pixels: *mut u32,
    width: i32,
    height: i32,
}

impl Canvas {
    fn new(width: i32, height: i32) -> Canvas {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut pixels = null_mut();
        // SAFETY: the DIB is created for a new memory DC and selected into it.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
            SelectObject(dc, bitmap);
            let canvas = Canvas {
                dc,
                bitmap,
                pixels: pixels.cast(),
                width,
                height,
            };
            core::slice::from_raw_parts_mut(canvas.pixels, (width * height) as usize)
                .fill(0x00FF_FFFF);
            canvas
        }
    }

    fn pixels(&self) -> &[u32] {
        // SAFETY: the DIB holds width × height pixels.
        unsafe { core::slice::from_raw_parts(self.pixels, (self.width * self.height) as usize) }
    }

    /// Whether a pixel has a strong hue, as no one-color glyph on white has.
    fn colorful(&self) -> bool {
        self.pixels().iter().any(|pixel| {
            let [b, g, r, _] = pixel.to_le_bytes();
            r.max(g).max(b) - r.min(g).min(b) > 100
        })
    }

    /// The columns with painted pixels.
    fn painted_columns(&self) -> Option<(i32, i32)> {
        let columns: Vec<i32> = (0..self.width)
            .filter(|x| {
                (0..self.height).any(|y| {
                    self.pixels()[(y * self.width + x) as usize] & 0x00FF_FFFF != 0x00FF_FFFF
                })
            })
            .collect();
        Some((*columns.first()?, *columns.last()?))
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: both were created by this canvas.
        unsafe {
            DeleteDC(self.dc);
            DeleteObject(self.bitmap);
        }
    }
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn emoji_are_drawn_in_their_colors() {
    let canvas = Canvas::new(120, 60);
    let width = draw(
        canvas.dc,
        20,
        40,
        &units("😀"),
        "Segoe UI Emoji",
        32.0,
        400,
        false,
        &[],
        0,
    )
    .unwrap();
    assert!(width > 20.0 && width < 60.0, "{width}");
    assert!(canvas.colorful());
    // Drawn where asked: from the start point, within its width.
    let (first, last) = canvas.painted_columns().unwrap();
    assert!(
        first >= 18 && last <= 20 + width.ceil() as i32 + 2,
        "{first} {last}"
    );
    assert_eq!(
        width,
        super::width(&units("😀"), "Segoe UI Emoji", 32.0, 400, false, &[]).unwrap()
    );
}

#[test]
fn glyphs_without_color_layers_take_the_text_color() {
    let canvas = Canvas::new(120, 60);
    let red = 0x0000_00FF;
    draw(
        canvas.dc,
        10,
        40,
        &units("#"),
        "Segoe UI Emoji",
        32.0,
        700,
        true,
        &[],
        red,
    )
    .unwrap();
    assert!(canvas.pixels().iter().any(|pixel| {
        let [b, g, r, _] = pixel.to_le_bytes();
        r > 200 && g < 100 && b < 100
    }));
}

#[test]
fn emoji_sequences_lay_out_as_one_emoji() {
    let single = width(&units("👨"), "Segoe UI Emoji", 20.0, 400, false, &[]).unwrap();
    let family = width(
        &units("👨\u{200d}👩\u{200d}👧"),
        "Segoe UI Emoji",
        20.0,
        400,
        false,
        &[],
    )
    .unwrap();
    assert!(family < single * 1.5, "{single} {family}");
}

#[test]
fn transformed_text_is_left_to_gdi_and_translations_are_followed() {
    let canvas = Canvas::new(120, 60);
    // SAFETY: the DC is live.
    unsafe {
        SetGraphicsMode(canvas.dc, GM_ADVANCED);
        let scaled = XFORM {
            eM11: 2.0,
            eM22: 2.0,
            ..XFORM::default()
        };
        SetWorldTransform(canvas.dc, &scaled);
    }
    assert!(
        draw(
            canvas.dc,
            0,
            20,
            &units("😀"),
            "Segoe UI Emoji",
            16.0,
            400,
            false,
            &[],
            0
        )
        .is_none()
    );
    assert!(!canvas.colorful());
    let moved = XFORM {
        eM11: 1.0,
        eM22: 1.0,
        eDx: 50.0,
        ..XFORM::default()
    };
    let mut kept = XFORM::default();
    // SAFETY: the DC is live.
    unsafe {
        SetWorldTransform(canvas.dc, &moved);
    }
    // A larger draw grows the kept render target.
    draw(
        canvas.dc,
        0,
        40,
        &units("😀😀"),
        "Segoe UI Emoji",
        36.0,
        400,
        false,
        &[],
        0,
    )
    .unwrap();
    // SAFETY: the DC is live.
    unsafe { GetWorldTransform(canvas.dc, &mut kept) };
    assert_eq!(kept.eDx, 50.0);
    let (first, _) = canvas.painted_columns().unwrap();
    assert!(first >= 48, "{first}");
}

#[test]
fn renderers_answer_only_their_interfaces() {
    let mut out = null_mut();
    let mut renderer = Renderer {
        vtable: &RENDERER_VTABLE,
        target: null_mut(),
        color: null_mut(),
        params: null_mut(),
        text_color: 0,
    };
    let this: *mut c_void = (&raw mut renderer).cast();
    // SAFETY: the renderer and outputs are live.
    unsafe {
        assert_eq!(
            renderer_query_interface(this, &IID_TEXT_RENDERER, &mut out),
            S_OK
        );
        assert_eq!(out, this);
        assert_eq!(
            renderer_query_interface(this, &IID_FACTORY, &mut out),
            E_NOINTERFACE
        );
        assert!(out.is_null());
        assert_eq!(renderer_reference(this), 1);
        let mut disabled = 1;
        renderer_snapping(this, null(), &mut disabled);
        assert_eq!(disabled, 0);
        let mut matrix = [0.0; 6];
        renderer_transform(this, null(), &mut matrix);
        assert_eq!(matrix, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let mut pixels = 0.0;
        renderer_pixels_per_dip(this, null(), &mut pixels);
        assert_eq!(pixels, 1.0);
        assert_eq!(
            renderer_draw_line(this, null(), 0.0, 0.0, null(), null_mut()),
            S_OK
        );
        assert_eq!(
            renderer_draw_inline_object(this, null(), 0.0, 0.0, null_mut(), 0, 0, null_mut()),
            S_OK
        );
    }
}

#[test]
fn woff2_fonts_unpack_into_fonts_gdi_loads() {
    let packed = include_bytes!("../tests/fixtures/octicons.woff2");
    let font = unpack_font(packed).expect("DirectWrite unpacks WOFF2");
    assert!(matches!(&font[..4], [0, 1, 0, 0] | b"OTTO" | b"true"));
    assert!(font.len() > packed.len());
    assert!(crate::register_font("Octicons Regular", 400, false, &font));
    assert_eq!(unpack_font(b"not a font"), None);
    assert_eq!(unpack_font(&font), None);
}

#[test]
fn opentype_features_apply_to_the_layout() {
    // Candara's figures are proportional unless tabular ones are asked for.
    let tabular = [(u32::from_le_bytes(*b"tnum"), 1)];
    let measure = |text: &str, features: &[(u32, u32)]| {
        width(&units(text), "Candara", 40.0, 400, false, features).unwrap()
    };
    assert!(measure("1111", &[]) < measure("0000", &[]));
    assert_eq!(measure("1111", &tabular), measure("0000", &tabular));
    // A value of zero turns a feature off.
    let off = [(u32::from_le_bytes(*b"tnum"), 0)];
    assert_eq!(measure("1111", &off), measure("1111", &[]));
}

#[test]
fn glyphs_of_fonts_loaded_from_memory_are_rasterized_as_from_their_files() {
    // ClearType coverage of a run in a system font, read from its file, and
    // in the same font loaded from memory under another name, as web fonts
    // are, match. Asking again reuses the face.
    let data = std::fs::read("C:/Windows/Fonts/arial.ttf").expect("Arial ships with Windows");
    assert!(crate::fonts::register_font(
        "Memory Arial",
        400,
        false,
        &data
    ));
    let covered = |family: &str| {
        let request = nativeboat_ir::Font {
            families: vec![family.to_owned()],
            size: 20.0,
            weight: 400,
            italic: false,
            features: Vec::new(),
        };
        let font = crate::fonts::font(&request, 1.0, false, false);
        // SAFETY: a memory DC for the test, with the font selected.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            SelectObject(dc, font);
            let glyphs = crate::fonts::simple_glyphs(dc, &units("Wave")).unwrap();
            let advances = [12.0; 4];
            let first = coverage(dc, &glyphs, &advances, 20.0, (2.0, 20.0), true).unwrap();
            let again = coverage(dc, &glyphs, &advances, 20.0, (2.0, 20.0), true).unwrap();
            DeleteDC(dc);
            assert_eq!(first.thirds, again.thirds);
            let RECT {
                left,
                top,
                right,
                bottom,
            } = first.bounds;
            ((left, top, right, bottom), first.thirds)
        }
    };
    let file = covered("Arial");
    assert!(file.1.iter().any(|third| *third > 0));
    assert_eq!(covered("Memory Arial"), file);
    // A space covers nothing.
    let font = crate::fonts::font(
        &nativeboat_ir::Font {
            families: vec!["Arial".to_owned()],
            size: 20.0,
            weight: 400,
            italic: false,
            features: Vec::new(),
        },
        1.0,
        false,
        false,
    );
    // SAFETY: a memory DC for the test, with the font selected.
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        SelectObject(dc, font);
        let space = crate::fonts::simple_glyphs(dc, &units(" ")).unwrap();
        let blank = coverage(dc, &space, &[6.0], 20.0, (2.0, 20.0), true).unwrap();
        assert!(blank.thirds.is_empty());
        DeleteDC(dc);
    }
}

/// The face GDI selects by a name, weight and slant, as faces are kept.
fn face_key(name: &str, weight: i32) -> FaceKey {
    let mut face = vec![0u16; 32];
    for (slot, unit) in face.iter_mut().zip(name.encode_utf16()) {
        *slot = unit;
    }
    (face, weight, 0)
}

/// A GDI font description of a face at a weight, 20 pixels high.
fn described(name: &str, weight: i32) -> LOGFONTW {
    let mut logfont = LOGFONTW {
        lfHeight: -20,
        lfWeight: weight,
        ..LOGFONTW::default()
    };
    logfont
        .lfFaceName
        .copy_from_slice(&face_key(name, weight).0);
    logfont
}

#[test]
fn faces_are_made_ahead_on_worker_threads() {
    // A face being made is waited for, and taken once; asking again while
    // it is made starts nothing more.
    prepare(&described("Arial", 401), false);
    prepare(&described("Arial", 401), false);
    assert!(READY.take(&face_key("Arial", 401)).is_some());
    assert!(READY.take(&face_key("Arial", 401)).is_none());
    // A font loaded from memory is read from a copy of its data.
    prepare(&described("Arial", 402), true);
    assert!(READY.take(&face_key("Arial", 402)).is_some());
    // Nothing is made for a font DirectWrite cannot read, as GDI's raster
    // System font.
    prepare(&described("System", 403), false);
    assert!(READY.take(&face_key("System", 403)).is_none());
    // Text in a font whose face is made uses that face.
    let logfont = described("Arial", 404);
    prepare(&logfont, false);
    // SAFETY: a memory DC for the test, with the font selected.
    unsafe {
        let font = CreateFontIndirectW(&logfont);
        let dc = CreateCompatibleDC(null_mut());
        SelectObject(dc, font);
        let glyphs = crate::fonts::simple_glyphs(dc, &units("Wave")).unwrap();
        assert!(coverage(dc, &glyphs, &[12.0; 4], 20.0, (2.0, 20.0), true).is_some());
        DeleteDC(dc);
        DeleteObject(font);
    }
    assert!(READY.take(&face_key("Arial", 404)).is_none());
    // A font no worker read is read by GDI interop.
    let logfont = described("Arial", 405);
    // SAFETY: a memory DC for the test, with the font selected.
    unsafe {
        let font = CreateFontIndirectW(&logfont);
        let dc = CreateCompatibleDC(null_mut());
        SelectObject(dc, font);
        let glyphs = crate::fonts::simple_glyphs(dc, &units("Wave")).unwrap();
        assert!(coverage(dc, &glyphs, &[12.0; 4], 20.0, (2.0, 20.0), true).is_some());
        DeleteDC(dc);
        DeleteObject(font);
    }
}
