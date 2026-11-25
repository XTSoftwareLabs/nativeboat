use super::*;
use nativeboat_ir::PathSegment::{Close, Cubic, Line, Move};
use shimforge::{Session, mock};

const RED: Color = Color::rgb(255, 0, 0);
const BLUE: Color = Color::rgb(0, 0, 255);
const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

fn rect(x: f32, y: f32, width: f32, height: f32) -> Vec<PathSegment> {
    vec![
        Move(x, y),
        Line(x + width, y),
        Line(x + width, y + height),
        Line(x, y + height),
        Close,
    ]
}

fn pixel(key: u64, x: i32, y: i32) -> [u8; 4] {
    canvas_read(key, x, y, 1, 1).try_into().unwrap()
}

fn near(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (i16::from(*a) - i16::from(e)).abs() <= 3)
}

fn fill(path: Vec<PathSegment>, paint: CanvasPaint) -> CanvasCommand {
    CanvasCommand::Fill {
        path,
        paint,
        rule: FillRule::NonZero,
        alpha: 1.0,
        copy: false,
    }
}

fn stroke(
    path: Vec<PathSegment>,
    cap: LineCap,
    join: LineJoin,
    dash: Vec<f32>,
    transform: [f32; 6],
) -> CanvasCommand {
    CanvasCommand::Stroke {
        path,
        paint: CanvasPaint::Color(RED),
        width: 4.0,
        cap,
        join,
        miter: 10.0,
        dash,
        offset: 0.0,
        transform,
        alpha: 1.0,
        copy: false,
    }
}

fn font(families: &[&str]) -> Font {
    Font {
        families: families.iter().map(|family| (*family).to_owned()).collect(),
        size: 20.0,
        weight: 400,
        italic: false,
        features: Vec::new(),
    }
}

/// Whether any pixel of a rectangle is not transparent.
fn inked(key: u64, x: i32, y: i32, width: u32, height: u32) -> bool {
    canvas_read(key, x, y, width, height)
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] > 0)
}

#[test]
fn shapes_fill_clear_and_resize() {
    let key = 9001;
    canvas_draw(
        key,
        20,
        10,
        &[
            fill(rect(0.0, 0.0, 10.0, 10.0), CanvasPaint::Color(RED)),
            CanvasCommand::Fill {
                path: rect(10.0, 0.0, 10.0, 10.0),
                paint: CanvasPaint::Color(BLUE),
                rule: FillRule::EvenOdd,
                alpha: 0.5,
                copy: false,
            },
        ],
    );
    assert_eq!(pixel(key, 5, 5), [255, 0, 0, 255]);
    assert!(near(pixel(key, 15, 5), [0, 0, 255, 128]));
    canvas_draw(
        key,
        20,
        10,
        &[
            CanvasCommand::Clear {
                path: rect(0.0, 0.0, 5.0, 10.0),
            },
            CanvasCommand::Fill {
                path: rect(10.0, 0.0, 10.0, 10.0),
                paint: CanvasPaint::Color(Color::rgba(0, 255, 0, 0.5)),
                rule: FillRule::NonZero,
                alpha: 1.0,
                copy: true,
            },
        ],
    );
    assert_eq!(pixel(key, 2, 2), CLEAR);
    assert_eq!(pixel(key, 7, 2), [255, 0, 0, 255]);
    assert!(near(pixel(key, 15, 5), [0, 255, 0, 128]));
    canvas_draw(key, 30, 10, &[]);
    assert_eq!(pixel(key, 7, 2), CLEAR, "a new size starts transparent");
    canvas_draw(
        key,
        0,
        10,
        &[fill(rect(0.0, 0.0, 1.0, 1.0), CanvasPaint::Color(RED))],
    );
    assert!(bitmap(key).is_none());
    canvas_draw(key, 0, 0, &[]);
    assert_eq!(pixel(key, 0, 0), CLEAR);
    canvas_release(key);
}

#[test]
fn strokes_with_caps_dashes_and_transforms() {
    let key = 9002;
    canvas_draw(
        key,
        40,
        40,
        &[
            stroke(
                vec![Move(0.0, 5.0), Line(40.0, 5.0)],
                LineCap::Butt,
                LineJoin::Miter,
                Vec::new(),
                IDENTITY,
            ),
            stroke(
                vec![Move(2.0, 15.0), Line(40.0, 15.0)],
                LineCap::Round,
                LineJoin::Round,
                vec![10.0, 10.0],
                IDENTITY,
            ),
            // The pen is shaped in the stroke's space: twice as wide here.
            stroke(
                vec![Move(0.0, 30.0), Cubic(10.0, 30.0, 20.0, 30.0, 30.0, 30.0)],
                LineCap::Square,
                LineJoin::Bevel,
                Vec::new(),
                [2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
            ),
            stroke(
                vec![Move(0.0, 38.0), Line(40.0, 38.0)],
                LineCap::Butt,
                LineJoin::Miter,
                Vec::new(),
                [0.0; 6],
            ),
            CanvasCommand::Stroke {
                path: vec![Move(35.0, 0.0), Line(35.0, 25.0)],
                paint: CanvasPaint::Radial {
                    start: (0.0, 0.0, 0.0),
                    end: (0.0, 0.0, 1.0),
                    stops: vec![(0.0, RED), (1.0, BLUE)],
                },
                width: 2.0,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter: 4.0,
                dash: vec![1.0],
                offset: 1.0,
                transform: IDENTITY,
                alpha: 1.0,
                copy: true,
            },
        ],
    );
    assert_eq!(pixel(key, 20, 5), [255, 0, 0, 255]);
    assert_eq!(pixel(key, 20, 9), CLEAR);
    assert_eq!(pixel(key, 6, 15), [255, 0, 0, 255], "a dash");
    assert_eq!(pixel(key, 17, 15), CLEAR, "a gap");
    assert_eq!(pixel(key, 10, 33), [255, 0, 0, 255], "a widened pen");
    assert_eq!(pixel(key, 10, 38), CLEAR, "a degenerate transform");
    assert!(
        inked(key, 34, 0, 2, 25),
        "a radial stroke uses its outer color"
    );
    canvas_release(key);
}

#[test]
fn gradients_pad_beyond_their_ends() {
    let key = 9003;
    let stops = vec![(1.5, BLUE), (-1.0, RED)];
    canvas_draw(
        key,
        100,
        30,
        &[
            fill(
                rect(0.0, 0.0, 100.0, 10.0),
                CanvasPaint::Linear {
                    start: (20.0, 0.0),
                    end: (80.0, 0.0),
                    stops: stops.clone(),
                },
            ),
            fill(
                rect(0.0, 10.0, 100.0, 10.0),
                CanvasPaint::Radial {
                    start: (50.0, 15.0, 0.0),
                    end: (50.0, 15.0, 20.0),
                    stops,
                },
            ),
            fill(
                rect(0.0, 20.0, 100.0, 10.0),
                CanvasPaint::Linear {
                    start: (0.0, 0.0),
                    end: (10.0, 0.0),
                    stops: Vec::new(),
                },
            ),
        ],
    );
    assert!(
        near(pixel(key, 5, 5), [255, 0, 0, 255]),
        "{:?}",
        pixel(key, 5, 5)
    );
    assert!(near(pixel(key, 95, 5), [0, 0, 255, 255]));
    let middle = pixel(key, 50, 5);
    assert!(middle[0] > 60 && middle[2] > 60, "{middle:?}");
    let center = pixel(key, 50, 15);
    assert!(center[0] > 200 && center[2] < 30, "{center:?}");
    assert!(
        near(pixel(key, 95, 15), [0, 0, 255, 255]),
        "outside the end circle"
    );
    assert_eq!(pixel(key, 50, 25), CLEAR, "no stops paint nothing");
    canvas_release(key);
}

#[test]
fn text_is_filled_and_outlined() {
    let key = 9004;
    let mut bold = font(&["NoSuchFont", "serif"]);
    bold.weight = 700;
    bold.italic = true;
    canvas_draw(
        key,
        200,
        100,
        &[
            CanvasCommand::Text {
                text: "Hi".to_owned(),
                x: 10.0,
                y: 30.0,
                anchor: TextAnchor::Left,
                font: font(&["Segoe UI"]),
                paint: CanvasPaint::Color(RED),
                stroke: None,
                transform: IDENTITY,
                alpha: 1.0,
            },
            CanvasCommand::Text {
                text: "Hi".to_owned(),
                x: 100.0,
                y: 30.0,
                anchor: TextAnchor::Center,
                font: bold,
                paint: CanvasPaint::Color(BLUE),
                stroke: Some(1.0),
                transform: IDENTITY,
                alpha: 1.0,
            },
            CanvasCommand::Text {
                text: "Hi".to_owned(),
                x: 190.0,
                y: 80.0,
                anchor: TextAnchor::Right,
                font: font(&["monospace"]),
                paint: CanvasPaint::Color(RED),
                stroke: None,
                transform: IDENTITY,
                alpha: 0.5,
            },
        ],
    );
    assert!(inked(key, 10, 10, 30, 25), "left-aligned text");
    assert!(!inked(key, 0, 40, 200, 20));
    assert!(inked(key, 85, 10, 30, 25), "centered outline");
    assert!(inked(key, 150, 60, 40, 25), "right-aligned text");
    let (width, ascent, descent) = measure_text(&font(&["sans-serif"]), "Hello");
    assert!(width > 20.0 && ascent > descent && descent > 0.0);
    let (wider, _, _) = measure_text(&font(&["system-ui"]), "Hello world");
    assert!(wider > width);
    assert!(measure_text(&font(&["NoSuchFont"]), "x").0 > 0.0);
    canvas_release(key);
}

#[test]
fn images_clips_and_states() {
    let (key, other) = (9005, 9006);
    crate::render::add_image("canvas-test.bmp", &crate::render::tests::bitmap_file());
    canvas_draw(
        other,
        10,
        10,
        &[fill(rect(0.0, 0.0, 10.0, 10.0), CanvasPaint::Color(BLUE))],
    );
    let image = |source, to, alpha| CanvasCommand::Image {
        source,
        from: Rect::new(0.0, 0.0, 1.0, 1.0),
        to,
        transform: IDENTITY,
        alpha,
    };
    canvas_draw(
        key,
        40,
        40,
        &[
            image(
                CanvasSource::Url("canvas-test.bmp".to_owned()),
                Rect::new(0.0, 0.0, 10.0, 10.0),
                1.0,
            ),
            image(
                CanvasSource::Canvas(other),
                Rect::new(10.0, 0.0, 10.0, 10.0),
                0.5,
            ),
            image(
                CanvasSource::Canvas(key),
                Rect::new(20.0, 0.0, 10.0, 10.0),
                1.0,
            ),
            image(
                CanvasSource::Url("missing.png".to_owned()),
                Rect::new(30.0, 0.0, 10.0, 10.0),
                1.0,
            ),
            CanvasCommand::Save,
            CanvasCommand::Clip {
                path: rect(0.0, 20.0, 10.0, 20.0),
                rule: FillRule::EvenOdd,
            },
            fill(rect(0.0, 20.0, 40.0, 10.0), CanvasPaint::Color(RED)),
            CanvasCommand::Restore,
            CanvasCommand::Restore,
            fill(rect(0.0, 30.0, 40.0, 10.0), CanvasPaint::Color(RED)),
        ],
    );
    assert_eq!(pixel(key, 2, 2), [0, 0, 255, 255], "the image's blue pixel");
    assert!(
        near(pixel(key, 15, 5), [0, 0, 255, 128]),
        "the other canvas"
    );
    assert_eq!(pixel(key, 25, 5), CLEAR, "a canvas cannot draw itself");
    assert_eq!(pixel(key, 35, 5), CLEAR);
    assert_eq!(pixel(key, 5, 25), [255, 0, 0, 255]);
    assert_eq!(pixel(key, 25, 25), CLEAR, "clipped");
    assert_eq!(pixel(key, 25, 35), [255, 0, 0, 255], "restored");
    canvas_release(key);
    canvas_release(other);
}

#[test]
fn pixels_are_read_written_and_encoded() {
    let key = 9007;
    canvas_draw(key, 4, 4, &[]);
    let data = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
    ];
    canvas_write(key, 1, 1, 2, 2, &data);
    let read = canvas_read(key, 1, 1, 2, 2);
    assert_eq!(read[..12], data[..12]);
    assert!(near(read[12..].try_into().unwrap(), [255, 255, 255, 128]));
    let corner = canvas_read(key, 0, 0, 2, 2);
    assert_eq!(corner[12..], [255, 0, 0, 255], "partly outside");
    assert_eq!(canvas_read(key, -1, -1, 1, 1), CLEAR);
    assert_eq!(canvas_read(key, 10, 10, 1, 1), CLEAR);
    canvas_write(key, 1, 1, 1, 1, &[]);
    assert_eq!(pixel(key, 1, 1), CLEAR, "missing bytes are transparent");
    let png = canvas_encode(key, false).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    assert_eq!(
        crate::render::add_image("encoded.png", &png),
        Some((4.0, 4.0))
    );
    let jpeg = canvas_encode(key, true).unwrap();
    assert!(jpeg.starts_with(&[0xFF, 0xD8]) && jpeg.ends_with(&[0xFF, 0xD9]));
    assert_eq!(canvas_read(9999, 0, 0, 1, 1), CLEAR);
    canvas_write(9999, 0, 0, 1, 1, &[1, 2, 3, 4]);
    assert_eq!(canvas_encode(9999, false), None);
    canvas_release(key);
}

#[test]
fn platform_failures_are_survived() {
    let key = 9008;
    canvas_draw(key, 2, 2, &[]);
    let mut session = Session::new();
    let stream = mock!(
        session,
        CreateStreamOnHGlobal,
        unsafe extern "system" fn(
            HGLOBAL,
            windows_sys::core::BOOL,
            *mut *mut core::ffi::c_void,
        ) -> HRESULT
    );
    stream.expect().returning(|_, _, _| -1);
    assert_eq!(canvas_encode(key, false), None);
    drop(session);
    let mut session = Session::new();
    let save = mock!(
        session,
        GdipSaveImageToStream,
        unsafe extern "system" fn(
            *mut GpImage,
            *mut core::ffi::c_void,
            *const GUID,
            *const EncoderParameters,
        ) -> Status
    );
    save.expect().returning(|_, _, _, _| GenericError);
    assert_eq!(canvas_encode(key, false), None);
    let lock = mock!(
        session,
        GdipBitmapLockBits,
        unsafe extern "system" fn(
            *mut GpBitmap,
            *const windows_sys::Win32::Graphics::GdiPlus::Rect,
            u32,
            i32,
            *mut BitmapData,
        ) -> Status
    );
    lock.expect().returning(|_, _, _, _, _| GenericError);
    assert_eq!(canvas_read(key, 0, 0, 1, 1), CLEAR);
    let create = mock!(
        session,
        GdipCreateBitmapFromScan0,
        unsafe extern "system" fn(i32, i32, i32, i32, *const u8, *mut *mut GpBitmap) -> Status
    );
    create.expect().returning(|_, _, _, _, _, _| OutOfMemory);
    canvas_draw(key, 3, 3, &[]);
    assert!(bitmap(key).is_none());
    assert_eq!(measure_text(&font(&["serif"]), "x"), (0.0, 0.0, 0.0));
    drop(session);
}

#[test]
fn images_are_encoded_redrawn_and_made_from_pixels() {
    let pixels = [
        255, 0, 0, 255, 0, 0, 255, 255, 0, 255, 0, 255, 255, 255, 255, 255,
    ];
    assert_eq!(
        image_from_pixels("pixels.png", (2, 2), &pixels),
        Some((2.0, 2.0))
    );
    let (width, height, read) = image_pixels("pixels.png").unwrap();
    assert_eq!((width, height, read.as_slice()), (2, 2, pixels.as_slice()));
    assert!(
        image_encode("pixels.png", false)
            .unwrap()
            .starts_with(b"\x89PNG")
    );
    assert!(
        image_encode("pixels.png", true)
            .unwrap()
            .starts_with(&[0xFF, 0xD8])
    );
    assert_eq!(
        image_draw(
            "pixels.png",
            Rect::new(1.0, 0.0, 1.0, 2.0),
            (1, 2),
            "cropped.png"
        ),
        Some((1.0, 2.0))
    );
    let (_, _, cropped) = image_pixels("cropped.png").unwrap();
    assert_eq!(
        cropped,
        [0, 0, 255, 255, 255, 255, 255, 255],
        "an exact crop"
    );
    assert_eq!(
        image_draw(
            "pixels.png",
            Rect::new(0.0, 0.0, 2.0, 2.0),
            (4, 4),
            "large.png"
        ),
        Some((4.0, 4.0))
    );
    let (_, _, large) = image_pixels("large.png").unwrap();
    assert_eq!(large[..4], [255, 0, 0, 255], "the scaled corner");
    assert_eq!(
        image_from_pixels("short.png", (1, 1), &[]),
        Some((1.0, 1.0))
    );
    assert_eq!(image_pixels("short.png").unwrap().2, CLEAR);
    assert_eq!(image_from_pixels("empty.png", (0, 1), &[]), None);
    assert_eq!(
        image_draw(
            "pixels.png",
            Rect::new(0.0, 0.0, 1.0, 1.0),
            (0, 0),
            "none.png"
        ),
        None
    );
    assert_eq!(
        image_draw(
            "missing.png",
            Rect::new(0.0, 0.0, 1.0, 1.0),
            (1, 1),
            "none.png"
        ),
        None
    );
    assert_eq!(image_encode("missing.png", false), None);
    assert_eq!(image_pixels("missing.png"), None);
}

#[test]
fn webp_images_are_decoded_without_a_system_codec() {
    crate::init().unwrap();
    // A lossless 2x1 WebP: opaque red, then blue at half opacity.
    let webp = [
        82, 73, 70, 70, 32, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 76, 19, 0, 0, 0, 47, 1, 0, 0, 16,
        15, 16, 251, 63, 255, 15, 252, 143, 10, 21, 136, 232, 127, 0, 0,
    ];
    assert_eq!(
        crate::render::add_image("image.webp", &webp),
        Some((2.0, 1.0))
    );
    let (width, height, pixels) = crate::image_pixels("image.webp").unwrap();
    assert_eq!((width, height), (2, 1));
    assert_eq!(pixels[..4], [255, 0, 0, 255]);
    assert_eq!((pixels[6], pixels[7]), (255, 128));
    assert_eq!(crate::render::add_image("broken.webp", &webp[..20]), None);
    assert_eq!(crate::render::add_image("text", b"not an image"), None);
}
