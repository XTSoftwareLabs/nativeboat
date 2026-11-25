use super::*;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::io::Write;

fn chunk(kind: &[u8; 4], data: &[u8], png: &mut Vec<u8>) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    png.extend_from_slice(&crc32(&png[png.len() - data.len() - 4..]).to_be_bytes());
}

fn png(color_type: u8, width: u32, height: u32, rows: &[u8]) -> Vec<u8> {
    let mut png = PNG_SIGNATURE.to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, color_type, 0, 0, 0]);
    chunk(b"IHDR", &header, &mut png);
    if color_type == 3 {
        chunk(b"PLTE", &[255, 0, 0, 0, 255, 0], &mut png);
        chunk(b"tRNS", &[128, 255], &mut png);
    } else if color_type == 0 {
        chunk(b"tRNS", &[0, 255], &mut png);
    } else if color_type == 2 {
        chunk(b"tRNS", &[0, 1, 0, 2, 0, 3], &mut png);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(rows).unwrap();
    chunk(b"IDAT", &encoder.finish().unwrap(), &mut png);
    chunk(b"IEND", &[], &mut png);
    png
}

#[allow(clippy::too_many_arguments)]
fn custom_png(
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    compression: u8,
    filter_method: u8,
    interlace: u8,
    chunks: &[(&[u8; 4], &[u8])],
) -> Vec<u8> {
    let mut output = PNG_SIGNATURE.to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[bit_depth, color_type, compression, filter_method, interlace]);
    chunk(b"IHDR", &header, &mut output);
    for (kind, data) in chunks {
        chunk(kind, data, &mut output);
    }
    output
}

#[test]
fn decodes_rgba_and_rgb_pngs_and_builds_top_down_bgra_dib() {
    let image = decode_png(&png(6, 2, 1, &[0, 10, 20, 30, 40, 50, 60, 70, 80])).unwrap();
    assert_eq!(image.rgba, [10, 20, 30, 40, 50, 60, 70, 80]);
    let dib = rgba_to_dibv5(2, 1, &image.rgba).unwrap();
    assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 124);
    assert_eq!(u32::from_le_bytes(dib[8..12].try_into().unwrap()), u32::MAX);
    assert_eq!(&dib[124..], &[30, 20, 10, 40, 70, 60, 50, 80]);
    let rgb = decode_png(&png(2, 2, 1, &[0, 1, 2, 3, 4, 5, 6])).unwrap();
    assert_eq!(rgb.rgba, [1, 2, 3, 0, 4, 5, 6, 255]);
}

#[test]
fn rejects_bad_png_chunks_dimensions_filters_and_pixel_buffers() {
    assert!(decode_png(b"not png").is_err());
    let mut bad_crc = png(6, 2, 1, &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
    bad_crc[29] ^= 1;
    assert!(decode_png(&bad_crc).is_err());
    assert!(decode_png(&png(6, 2, 1, &[5, 1, 2, 3, 4, 5, 6, 7, 8])).is_err());
    assert!(rgba_to_dibv5(0, 1, &[]).is_err());
    assert!(rgba_to_dibv5(1, 1, &[0; 3]).is_err());
    assert!(rgba_to_dibv5(u32::MAX, 1, &[]).is_err());
    assert!(rgba_to_dibv5(i32::MAX as u32 + 1, 0, &[]).is_err());
}

#[test]
fn reconstructs_scanline_filters_and_gray_palette_pixels() {
    let sub = decode_png(&png(0, 2, 1, &[1, 10, 10])).unwrap();
    assert_eq!(sub.rgba, [10, 10, 10, 255, 20, 20, 20, 255]);
    let average = decode_png(&png(0, 2, 1, &[3, 4, 6])).unwrap();
    assert_eq!(average.rgba, [4, 4, 4, 255, 8, 8, 8, 255]);
    let paeth_image = decode_png(&png(0, 2, 1, &[4, 4, 4])).unwrap();
    assert_eq!(paeth_image.rgba, [4, 4, 4, 255, 8, 8, 8, 255]);
    let up = decode_png(&png(0, 2, 2, &[0, 10, 20, 2, 1, 1])).unwrap();
    assert_eq!(
        up.rgba,
        [
            10, 10, 10, 255, 20, 20, 20, 255, 11, 11, 11, 255, 21, 21, 21, 255
        ]
    );
    let gray_alpha = decode_png(&png(4, 1, 1, &[0, 31, 127])).unwrap();
    assert_eq!(gray_alpha.rgba, [31, 31, 31, 127]);
    let transparent_gray = decode_png(&png(0, 1, 1, &[0, 255])).unwrap();
    assert_eq!(transparent_gray.rgba, [255, 255, 255, 0]);
    let palette = decode_png(&png(3, 2, 1, &[0, 0, 1])).unwrap();
    assert_eq!(palette.rgba, [255, 0, 0, 128, 0, 255, 0, 255]);
    assert_eq!(paeth(10, 0, 9), 0);
}

#[test]
fn rejects_incomplete_unsupported_and_malformed_png_structures() {
    assert!(decode_png(&PNG_SIGNATURE[..]).is_err());

    let valid = png(6, 1, 1, &[0, 1, 2, 3, 4]);
    assert!(decode_png(&valid[..valid.len() - 2]).is_err());

    let mut missing_end = valid.clone();
    missing_end.truncate(missing_end.len() - 12);
    assert!(decode_png(&missing_end).is_err());

    assert!(decode_png(&custom_png(1, 1, 8, 6, 1, 0, 0, &[])).is_err());
    assert!(decode_png(&custom_png(1, 1, 8, 6, 0, 1, 0, &[])).is_err());
    assert!(decode_png(&custom_png(1, 1, 4, 6, 0, 0, 0, &[])).is_err());
    assert!(decode_png(&custom_png(1, 1, 8, 6, 0, 0, 1, &[])).is_err());
    assert!(decode_png(&custom_png(1, 1, 8, 1, 0, 0, 0, &[])).is_err());
    assert!(decode_png(&custom_png(0, 1, 8, 6, 0, 0, 0, &[])).is_err());

    let no_header = {
        let mut image = PNG_SIGNATURE.to_vec();
        chunk(b"IEND", &[], &mut image);
        image
    };
    assert!(decode_png(&no_header).is_err());
    let duplicate_header = custom_png(1, 1, 8, 6, 0, 0, 0, &[(b"IHDR", &[0; 13]), (b"IEND", &[])]);
    assert!(decode_png(&duplicate_header).is_err());
    let unknown_critical = custom_png(1, 1, 8, 6, 0, 0, 0, &[(b"ABCD", &[]), (b"IEND", &[])]);
    assert!(decode_png(&unknown_critical).is_err());

    let invalid_deflate = custom_png(1, 1, 8, 6, 0, 0, 0, &[(b"IDAT", b"bad"), (b"IEND", b"")]);
    assert!(decode_png(&invalid_deflate).is_err());
    assert!(decode_png(&png(6, 1, 1, &[0, 1, 2])).is_err());
    assert!(decode_png(&png(6, 1, 1, &[5, 1, 2, 3, 4])).is_err());
    let mut oversized_rows = ZlibEncoder::new(Vec::new(), Compression::default());
    oversized_rows.write_all(&vec![0; 1024]).unwrap();
    let oversized_scanline = custom_png(
        1,
        1,
        8,
        6,
        0,
        0,
        0,
        &[(b"IDAT", &oversized_rows.finish().unwrap()), (b"IEND", &[])],
    );
    assert!(decode_png(&oversized_scanline).is_err());

    let malformed_palette = custom_png(
        1,
        1,
        8,
        3,
        0,
        0,
        0,
        &[(b"PLTE", &[1, 2]), (b"IDAT", &[]), (b"IEND", &[])],
    );
    assert!(decode_png(&malformed_palette).is_err());
    let missing_palette = custom_png(1, 1, 8, 3, 0, 0, 0, &[(b"IDAT", &[]), (b"IEND", &[])]);
    assert!(decode_png(&missing_palette).is_err());

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&[0, 1]).unwrap();
    let indexed_rows = encoder.finish().unwrap();
    let invalid_palette_index = custom_png(
        1,
        1,
        8,
        3,
        0,
        0,
        0,
        &[
            (b"PLTE", &[1, 2, 3]),
            (b"IDAT", &indexed_rows),
            (b"IEND", &[]),
        ],
    );
    assert!(decode_png(&invalid_palette_index).is_err());
}

#[test]
fn encoded_pngs_decode_to_the_same_pixels() {
    let rgba = vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 8, 7, 6];
    let png = encode_png(2, 2, &rgba);
    assert_eq!(&png[..8], PNG_SIGNATURE);
    let image = decode_png(&png).unwrap();
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(image.rgba, rgba);
}

#[test]
fn bitmaps_decode_from_bottom_up_rgb_padded_and_masked_layouts() {
    // 24 bits, bottom-up, rows padded to four bytes: 1x2, blue over red.
    let mut dib = vec![0u8; 40];
    dib[0] = 40;
    dib[4] = 1;
    dib[8] = 2;
    dib[12] = 1;
    dib[14] = 24;
    dib.extend([0, 0, 255, 0, 255, 0, 0, 0]);
    let image = decode_dib(&dib).unwrap();
    assert_eq!(image.rgba, [0, 0, 255, 255, 255, 0, 0, 255]);
    // 32 bits with zero alpha bytes are opaque.
    let mut plain = vec![0u8; 40];
    plain[0] = 40;
    plain[4] = 1;
    plain[8] = 1;
    plain[14] = 32;
    plain.extend([1, 2, 3, 0]);
    assert_eq!(decode_dib(&plain).unwrap().rgba, [3, 2, 1, 255]);
    // Bitfield masks after a 40-byte header.
    let mut masked = plain.clone();
    masked[16] = 3;
    masked.truncate(40);
    for mask in [0x0000_00ff_u32, 0x0000_ff00, 0x00ff_0000] {
        masked.extend(mask.to_le_bytes());
    }
    masked.extend([1, 2, 3, 0]);
    assert_eq!(decode_dib(&masked).unwrap().rgba, [1, 2, 3, 255]);
    // Top-down DIBV5 with alpha, as writeImage produces.
    let written = rgba_to_dibv5(1, 1, &[10, 20, 30, 40]).unwrap();
    assert_eq!(decode_dib(&written).unwrap().rgba, [10, 20, 30, 40]);
    assert!(decode_dib(&[0; 8]).is_err());
}

fn fixture(name: &str) -> String {
    alloc::format!(
        "{}\\src\\electron_native_image\\{name}",
        env!("CARGO_MANIFEST_DIR").replace('/', "\\")
    )
}

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap()
}

fn rep(name: &str) -> Rep {
    Rep::from_straight(1.0, decode_png(&bytes(name)).unwrap())
}

/// Calls a method of an object.
fn call(realm: &mut Realm, target: &Value, name: &'static str, args: &[Value]) -> Completion {
    let function = ops::get_named(realm, target, name)?;
    ops::call(realm, &function, target.clone(), args)
}

fn message(realm: &mut Realm, thrown: Thrown) -> String {
    let message = ops::get_named(realm, &thrown.0, "message").unwrap();
    let name = ops::get_named(realm, &thrown.0, "name").unwrap();
    alloc::format!(
        "{}: {}",
        ops::to_string(realm, &name).unwrap().to_rust_lossy(),
        ops::to_string(realm, &message).unwrap().to_rust_lossy()
    )
}

fn object(realm: &Realm, entries: &[(&str, Value)]) -> Value {
    let object = realm.object();
    for (name, value) in entries {
        object.define(Key::name(name), Property::data(value.clone()));
    }
    Value::Object(object)
}

fn size(realm: &mut Realm, image: &Value, args: &[Value]) -> (f64, f64) {
    let size = call(realm, image, "getSize", args).unwrap();
    let number = |realm: &mut Realm, name| match ops::get_named(realm, &size, name).unwrap() {
        Value::Number(number) => number,
        _ => panic!("not a number"),
    };
    (number(realm, "width"), number(realm, "height"))
}

fn view(value: &Value) -> Vec<u8> {
    crate::typed_array_builtin::view_bytes(value).unwrap()
}

fn numbers(realm: &mut Realm, array: &Value) -> Vec<f64> {
    let length = ops::get_named(realm, array, "length").unwrap();
    let Value::Number(length) = length else {
        panic!("no length")
    };
    (0..length as u32)
        .map(
            |index| match ops::get(realm, array, &Key::Index(index)).unwrap() {
                Value::Number(number) => number,
                _ => panic!("not a number"),
            },
        )
        .collect()
}

fn buffer(realm: &Realm, bytes: Vec<u8>) -> Value {
    Value::Object(crate::typed_array_builtin::new_bytes(realm, bytes, None))
}

#[test]
fn resizing_gives_chromiums_pixels() {
    // Measured in Electron 38: `resize` of a 40×30 PNG, and of a 12×10 one
    // with alpha.
    let source = rep("src.png");
    let resized = |rep: &Rep, size, method| {
        resize::resize(
            &rep.pixels,
            (rep.width, rep.height),
            size,
            method,
            rep.opaque,
        )
    };
    assert_eq!(
        resized(&source, (13, 7), Method::Lanczos),
        bytes("resize-13x7.bgra")
    );
    assert_eq!(
        resized(&source, (57, 43), Method::Hamming),
        bytes("resize-57-good.bgra")
    );
    assert_eq!(
        resized(&source, (80, 60), Method::Lanczos),
        bytes("resize-80x60.bgra")
    );
    let alpha = rep("alpha.png");
    assert!(!alpha.opaque);
    assert_eq!(
        resized(&alpha, (7, 5), Method::Lanczos),
        bytes("alpha-7x5.bgra")
    );
    assert_eq!(
        resized(&alpha, (30, 25), Method::Hamming),
        bytes("alpha-30x25-good.bgra")
    );
}

#[test]
fn pixels_are_premultiplied_and_written_back_as_skia_does() {
    let alpha = rep("alpha.png");
    assert_eq!(alpha.pixels, bytes("alpha.bgra"));
    // Electron's `toPNG` of the same picture.
    let written = decode_png(&bytes("alpha-electron.png")).unwrap();
    assert_eq!(alpha.straight(), written.rgba);
    // Halves round to even, as Skia's raster pipeline stores them.
    assert_eq!(unpremultiply(189, 238), 202);
    assert_eq!(unpremultiply(189, 210), 229);
    assert_eq!(unpremultiply(9, 0), 0);
    assert_eq!(premultiply(200, 128), 100);
    // Opaque pictures are written as RGB, as Chromium writes them.
    let opaque = decode_png(&rep("src.png").png()).unwrap();
    assert!(opaque.opaque);
    assert_eq!(rep("src.png").png()[25], 2);
    assert_eq!(alpha.png()[25], 6);
    assert!(Rep { width: 0, ..alpha }.png().is_empty());
}

fn near(left: &[u8], right: &[u8], tolerance: u8) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.abs_diff(*b) <= tolerance)
}

#[test]
fn jpegs_are_read_and_written_through_windows() {
    // libjpeg-turbo and Windows' decoder differ by rounding only.
    for (file, expected) in [
        ("photo.jpg", "photo.bgra"),
        ("photo420.jpg", "photo420.bgra"),
    ] {
        let (width, height, pixels) = windows::decode_jpeg(&bytes(file)).unwrap();
        assert_eq!((width, height), (40, 30));
        assert!(near(&pixels, &bytes(expected), 2), "{file}");
    }
    assert!(windows::decode_jpeg(&bytes("src.png")).is_none());
    assert!(windows::decode_jpeg(b"not a picture").is_none());
    // Written 4:2:0 with the standard tables, as Chromium writes them.
    let source = rep("src.png");
    let jpeg = windows::encode_jpeg(40, 30, &source.pixels, 80).unwrap();
    assert_eq!(jpeg[..2], [0xff, 0xd8]);
    let (_, _, decoded) = windows::decode_jpeg(&jpeg).unwrap();
    let expected = bytes("jpeg80.bgra");
    let difference: u32 = decoded
        .iter()
        .zip(&expected)
        .map(|(a, b)| u32::from(a.abs_diff(*b)))
        .sum();
    assert!(difference / (expected.len() as u32) < 3, "{difference}");
    let small = windows::encode_jpeg(40, 30, &source.pixels, 0).unwrap();
    assert!(small.len() < jpeg.len());
}

#[test]
fn icons_and_thumbnails_are_read_as_electron_reads_them() {
    // `LoadImage` scales the 32-pixel icon to 256 by repeating pixels.
    let (width, height, pixels) = windows::load_icon(&fixture("icon.ico"), 256).unwrap();
    assert_eq!((width, height), (256, 256));
    let at = |x: usize, y: usize| &pixels[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4];
    assert_eq!(at(0, 100), [0, 0, 255, 255]);
    assert_eq!(at(255, 255), [62, 62, 0, 255]);
    assert!(windows::load_icon(&fixture("missing.ico"), 256).is_none());
    // Explorer's thumbnail fits the width asked for, even past the picture's
    // own size: Electron's 40×30 photo gives 100×75 at 100.
    let (width, height, _) = windows::thumbnail(&fixture("src.png"), 16).unwrap();
    assert_eq!((width, height), (16, 12));
    let (width, height, _) = windows::thumbnail(&fixture("photo.jpg"), 100).unwrap();
    assert_eq!((width, height), (100, 75));
    assert_eq!(
        windows::thumbnail(&fixture("missing.png"), 16),
        Err("Failed to create IShellItem from the given path")
    );
}

#[test]
fn files_give_their_scale_factors_and_icons() {
    let reps = reps_from_path(&fixture("multi.png"));
    let scales: Vec<f32> = reps.iter().map(|rep| rep.scale).collect();
    assert_eq!(scales, [1.0, 2.0, 1.5]);
    // A file named with a scale factor is that scale alone.
    let reps = reps_from_path(&fixture("only@2x.png"));
    assert_eq!(
        (reps.len(), reps[0].scale, reps[0].size()),
        (1, 2.0, (10, 10))
    );
    assert_eq!(suffix_scale("a\\b@shadowx.png"), Some(1.0));
    assert_eq!(suffix_scale("a\\b@2x"), Some(2.0));
    assert_eq!(suffix_scale("a@2x\\b.png"), None);
    assert_eq!(with_suffix("a.b\\c", "@2x"), "a.b\\c@2x");
    let icon = reps_from_path(&fixture("icon.ico"));
    assert_eq!((icon[0].width, icon[0].height), (256, 256));
    assert!(reps_from_path(&fixture("missing.png")).is_empty());
    let jpeg = reps_from_path(&fixture("photo.jpg"));
    assert!(jpeg[0].opaque);
}

#[test]
fn data_urls_are_parsed_as_chromium_parses_them() {
    let png = bytes("src.png");
    let base64 = crate::encoding::base64_encode(&png, false);
    let size = |url: &str| rep_from_data_url(url, 1.0).map(|rep| (rep.width, rep.height));
    assert_eq!(
        size(&alloc::format!("DATA:IMAGE/PNG;BASE64,{base64}")),
        Some((40, 30))
    );
    assert_eq!(
        size(&alloc::format!(
            "data:image/png;charset=utf-8;base64,{base64}"
        )),
        Some((40, 30))
    );
    let spaced: String = base64.chars().flat_map(|c| [c, ' ']).collect();
    assert_eq!(
        size(&alloc::format!("data:image/png;base64,{spaced}")),
        Some((40, 30))
    );
    let escaped: String = png
        .iter()
        .map(|byte| alloc::format!("%{byte:02x}"))
        .collect();
    assert_eq!(
        size(&alloc::format!("data:image/png,{escaped}")),
        Some((40, 30))
    );
    assert_eq!(
        size(&alloc::format!("data:image/gif;base64,{base64}")),
        None
    );
    assert_eq!(
        size(&alloc::format!("data:image/jpeg;base64,{base64}")),
        None
    );
    assert_eq!(size(&alloc::format!("data:;base64,{base64}")), None);
    assert_eq!(size("http://example.com/a.png"), None);
    assert_eq!(size("data:image/png;base64"), None);
    let jpeg = crate::encoding::base64_encode(&bytes("photo.jpg"), false);
    assert_eq!(
        size(&alloc::format!("data:image/jpeg;base64,{jpeg}")),
        Some((40, 30))
    );
    assert_eq!(percent_decoded(b"%41%4g%"), b"A%4g%");
}

#[test]
fn images_have_electrons_shape() {
    let realm = &mut Realm::new();
    let module = Value::Object(module(realm));
    let empty = call(realm, &module, "createEmpty", &[]).unwrap();
    let Value::Object(image) = &empty else {
        panic!("not an object")
    };
    let names: Vec<String> = image
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(names, METHODS);
    let constructor = ops::get_named(realm, &empty, "constructor").unwrap();
    let name = ops::get_named(realm, &constructor, "name").unwrap();
    assert_eq!(
        ops::to_string(realm, &name).unwrap().to_rust_lossy(),
        "NativeImage"
    );
    assert!(matches!(
        ops::call(realm, &constructor, Value::Undefined, &[]),
        Ok(Value::Undefined)
    ));
    let made = ops::construct(realm, &constructor, &[]).unwrap();
    assert!(matches!(
        call(realm, &made, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    // The same functions serve every image.
    let other = call(realm, &module, "createEmpty", &[]).unwrap();
    assert!(crate::same_value(
        &ops::get_named(realm, &empty, "toPNG").unwrap(),
        &ops::get_named(realm, &other, "toPNG").unwrap()
    ));
    let create = ops::get_named(realm, &module, "createFromPath").unwrap();
    let name = ops::get_named(realm, &create, "name").unwrap();
    assert_eq!(ops::to_string(realm, &name).unwrap().to_rust_lossy(), "");
    // macOS's features are inert, as Electron on Windows keeps them.
    ops::set(
        realm,
        &empty,
        &Key::name("isMacTemplateImage"),
        Value::Boolean(true),
    )
    .unwrap();
    assert!(matches!(
        ops::get_named(realm, &empty, "isMacTemplateImage"),
        Ok(Value::Boolean(false))
    ));
    assert!(matches!(
        call(realm, &empty, "setTemplateImage", &[Value::Boolean(true)]),
        Ok(Value::Undefined)
    ));
    assert!(matches!(
        call(realm, &empty, "isTemplateImage", &[]),
        Ok(Value::Boolean(false))
    ));
    let thrown = call(realm, &empty, "setTemplateImage", &[]).unwrap_err();
    assert_eq!(
        message(realm, thrown),
        "TypeError: Insufficient number of arguments."
    );
    let thrown = call(realm, &empty, "setTemplateImage", &[Value::Number(1.0)]).unwrap_err();
    assert_eq!(
        message(realm, thrown),
        "TypeError: Error processing argument at index 0, conversion failure from "
    );
    let thrown = call(realm, &empty, "getNativeHandle", &[]).unwrap_err();
    assert_eq!(message(realm, thrown), "Error: Not implemented");
    let named = call(
        realm,
        &module,
        "createFromNamedImage",
        &[Value::str("NSActionTemplate")],
    )
    .unwrap();
    assert!(matches!(
        call(realm, &named, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    assert!(call(realm, &module, "createFromNamedImage", &[]).is_err());
    assert!(call(realm, &module, "createFromNamedImage", &[Value::Null]).is_err());
    // An empty image has no size, no pixels and an aspect ratio of 1.
    assert_eq!(size(realm, &empty, &[]), (0.0, 0.0));
    assert!(matches!(
        call(realm, &empty, "getAspectRatio", &[]),
        Ok(Value::Number(1.0))
    ));
    let png = call(realm, &empty, "toPNG", &[]).unwrap();
    assert!(view(&png).is_empty());
    let url = call(realm, &empty, "toDataURL", &[]).unwrap();
    assert_eq!(
        ops::to_string(realm, &url).unwrap().to_rust_lossy(),
        "data:image/png;base64,"
    );
    let jpeg = call(realm, &empty, "toJPEG", &[Value::Number(80.0)]).unwrap();
    assert!(view(&jpeg).is_empty());
    let factors = call(realm, &empty, "getScaleFactors", &[]).unwrap();
    assert!(numbers(realm, &factors).is_empty());
}

#[test]
fn images_are_made_from_files_buffers_bitmaps_and_urls() {
    let realm = &mut Realm::new();
    let module = Value::Object(module(realm));
    let path = Value::str(&fixture("src.png"));
    let image = call(realm, &module, "createFromPath", &[path]).unwrap();
    assert_eq!(size(realm, &image, &[]), (40.0, 30.0));
    let ratio = call(realm, &image, "getAspectRatio", &[]).unwrap();
    assert!(matches!(ratio, Value::Number(ratio) if ratio == f64::from(40.0_f32 / 30.0)));
    let multi = call(
        realm,
        &module,
        "createFromPath",
        &[Value::str(&fixture("multi.png"))],
    )
    .unwrap();
    let factors = call(realm, &multi, "getScaleFactors", &[]).unwrap();
    assert_eq!(numbers(realm, &factors), [1.0, 2.0, 1.5]);
    // The representation nearest the scale factor asked for.
    let options = object(realm, &[("scaleFactor", Value::Number(2.0))]);
    let green = call(realm, &multi, "toBitmap", &[options]).unwrap();
    assert_eq!(view(&green)[..4], [10, 200, 10, 255]);
    let missing = call(
        realm,
        &module,
        "createFromPath",
        &[Value::str(&fixture("missing.png"))],
    )
    .unwrap();
    assert!(matches!(
        call(realm, &missing, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    for (args, expected) in [
        (&[][..], "TypeError: Insufficient number of arguments."),
        (
            &[Value::Number(5.0)][..],
            "TypeError: Error processing argument at index 0, conversion failure from ",
        ),
    ] {
        let thrown = call(realm, &module, "createFromPath", args).unwrap_err();
        assert_eq!(message(realm, thrown), expected);
    }
    // Buffers: PNG, JPEG, or premultiplied pixels of the size given.
    let png = buffer(realm, bytes("src.png"));
    let scaled = object(realm, &[("scaleFactor", Value::Number(2.0))]);
    let image = call(realm, &module, "createFromBuffer", &[png, scaled]).unwrap();
    assert_eq!(size(realm, &image, &[]), (20.0, 15.0));
    let jpeg = buffer(realm, bytes("photo.jpg"));
    let image = call(realm, &module, "createFromBuffer", &[jpeg]).unwrap();
    assert_eq!(size(realm, &image, &[]), (40.0, 30.0));
    let raw: Vec<u8> = (0..48).map(|byte| (byte * 37 % 256) as u8).collect();
    let sized = object(
        realm,
        &[
            ("width", Value::Number(3.0)),
            ("height", Value::Number(2.0)),
        ],
    );
    let image = call(
        realm,
        &module,
        "createFromBuffer",
        &[buffer(realm, raw.clone()), sized.clone()],
    )
    .unwrap();
    assert_eq!(
        view(&call(realm, &image, "toBitmap", &[]).unwrap()),
        raw[..24]
    );
    let wide = object(
        realm,
        &[
            ("width", Value::Number(4.0)),
            ("height", Value::Number(4.0)),
        ],
    );
    let image = call(
        realm,
        &module,
        "createFromBuffer",
        &[buffer(realm, raw.clone()), wide],
    )
    .unwrap();
    assert!(matches!(
        call(realm, &image, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    let thrown = call(realm, &module, "createFromBuffer", &[Value::str("abc")]).unwrap_err();
    assert_eq!(
        message(realm, thrown),
        "Error: buffer must be a node Buffer"
    );
    assert!(call(realm, &module, "createFromBuffer", &[]).is_err());
    // Bitmaps: exactly their size, kept as given.
    let image = call(
        realm,
        &module,
        "createFromBitmap",
        &[buffer(realm, raw[..24].to_vec()), sized.clone()],
    )
    .unwrap();
    assert_eq!(
        view(&call(realm, &image, "toBitmap", &[]).unwrap()),
        raw[..24]
    );
    let half = object(
        realm,
        &[
            ("width", Value::Number(4.0)),
            ("height", Value::Number(4.0)),
            ("scaleFactor", Value::Number(2.0)),
        ],
    );
    let image = call(
        realm,
        &module,
        "createFromBitmap",
        &[buffer(realm, vec![0; 64]), half],
    )
    .unwrap();
    assert_eq!(size(realm, &image, &[]), (2.0, 2.0));
    for (args, expected) in [
        (
            vec![buffer(realm, raw.clone())],
            "TypeError: Insufficient number of arguments.",
        ),
        (
            vec![buffer(realm, raw.clone()), sized.clone()],
            "Error: invalid buffer size",
        ),
        (
            vec![Value::str("abc"), sized.clone()],
            "Error: buffer must be a node Buffer",
        ),
        (
            vec![
                buffer(realm, raw.clone()),
                object(realm, &[("height", Value::Number(2.0))]),
            ],
            "Error: width is required",
        ),
        (
            vec![
                buffer(realm, raw.clone()),
                object(realm, &[("width", Value::Number(2.0))]),
            ],
            "Error: height is required",
        ),
        (
            vec![buffer(realm, raw.clone()), Value::Number(1.0)],
            "TypeError: Error processing argument at index 1, conversion failure from ",
        ),
    ] {
        let thrown = call(realm, &module, "createFromBitmap", &args).unwrap_err();
        assert_eq!(message(realm, thrown), expected);
    }
    let none = object(
        realm,
        &[
            ("width", Value::Number(0.0)),
            ("height", Value::Number(0.0)),
        ],
    );
    let image = call(
        realm,
        &module,
        "createFromBitmap",
        &[buffer(realm, Vec::new()), none],
    )
    .unwrap();
    assert!(matches!(
        call(realm, &image, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    // Data URLs.
    let url = alloc::format!(
        "data:image/jpeg;base64,{}",
        crate::encoding::base64_encode(&bytes("photo.jpg"), false)
    );
    let image = call(realm, &module, "createFromDataURL", &[Value::str(&url)]).unwrap();
    assert_eq!(size(realm, &image, &[]), (40.0, 30.0));
    assert!(call(realm, &module, "createFromDataURL", &[]).is_err());
    assert!(call(realm, &module, "createFromDataURL", &[Value::Number(1.0)]).is_err());
}

#[test]
fn images_are_resized_cropped_and_written_as_electron_does() {
    let realm = &mut Realm::new();
    let module = Value::Object(module(realm));
    let image = call(
        realm,
        &module,
        "createFromPath",
        &[Value::str(&fixture("src.png"))],
    )
    .unwrap();
    let resize = |realm: &mut Realm, entries: &[(&str, Value)]| {
        let options = object(realm, entries);
        call(realm, &image, "resize", &[options])
    };
    // One side keeps the aspect ratio.
    let resized = resize(realm, &[("width", Value::Number(57.0))]).unwrap();
    assert_eq!(size(realm, &resized, &[]), (57.0, 43.0));
    let resized = resize(
        realm,
        &[
            ("height", Value::Number(15.0)),
            ("quality", Value::str("good")),
        ],
    )
    .unwrap();
    assert_eq!(size(realm, &resized, &[]), (20.0, 15.0));
    let good = resize(
        realm,
        &[
            ("width", Value::Number(57.0)),
            ("quality", Value::str("better")),
        ],
    )
    .unwrap();
    assert_eq!(
        view(&call(realm, &good, "toBitmap", &[]).unwrap()),
        bytes("resize-57-good.bgra")
    );
    // Sides that are not integers are not given.
    let same = resize(
        realm,
        &[
            ("width", Value::Number(2.6)),
            ("height", Value::Number(1.2)),
        ],
    )
    .unwrap();
    assert_eq!(size(realm, &same, &[]), (40.0, 30.0));
    // No size at all is an empty image; a zero side, an image of no size.
    let none = resize(
        realm,
        &[
            ("width", Value::Number(0.0)),
            ("height", Value::Number(0.0)),
        ],
    )
    .unwrap();
    assert!(matches!(
        call(realm, &none, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    let flat = resize(realm, &[("width", Value::Number(0.0))]).unwrap();
    assert!(matches!(
        call(realm, &flat, "isEmpty", &[]),
        Ok(Value::Boolean(false))
    ));
    assert_eq!(size(realm, &flat, &[]), (0.0, 0.0));
    let negative = resize(realm, &[("width", Value::Number(-5.0))]).unwrap();
    assert_eq!(size(realm, &negative, &[]), (0.0, 0.0));
    for (args, expected) in [
        (vec![], "TypeError: Insufficient number of arguments."),
        (
            vec![Value::Null],
            "TypeError: Error processing argument at index 0, conversion failure from null",
        ),
    ] {
        let thrown = call(realm, &image, "resize", &args).unwrap_err();
        assert_eq!(message(realm, thrown), expected);
    }
    let empty = call(realm, &module, "createEmpty", &[]).unwrap();
    let options = object(
        realm,
        &[
            ("width", Value::Number(5.0)),
            ("height", Value::Number(5.0)),
        ],
    );
    let resized = call(realm, &empty, "resize", std::slice::from_ref(&options)).unwrap();
    assert!(matches!(
        call(realm, &resized, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    // Crops are clipped to the image, in device-independent pixels.
    let crop = |realm: &mut Realm, image: &Value, rect: [f64; 4]| {
        let options = object(
            realm,
            &[
                ("x", Value::Number(rect[0])),
                ("y", Value::Number(rect[1])),
                ("width", Value::Number(rect[2])),
                ("height", Value::Number(rect[3])),
            ],
        );
        call(realm, image, "crop", &[options]).unwrap()
    };
    let cropped = crop(realm, &image, [1.5, 2.5, 3.5, 2.2]);
    assert_eq!(
        view(&call(realm, &cropped, "toBitmap", &[]).unwrap()),
        bytes("crop.bgra")
    );
    let corner = crop(realm, &image, [35.0, 25.0, 10.0, 10.0]);
    assert_eq!(size(realm, &corner, &[]), (5.0, 5.0));
    let outside = crop(realm, &image, [50.0, 50.0, 5.0, 5.0]);
    assert!(matches!(
        call(realm, &outside, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    let nothing = crop(realm, &empty, [0.0, 0.0, 5.0, 5.0]);
    assert!(matches!(
        call(realm, &nothing, "isEmpty", &[]),
        Ok(Value::Boolean(true))
    ));
    let double = call(
        realm,
        &module,
        "createFromPath",
        &[Value::str(&fixture("only@2x.png"))],
    )
    .unwrap();
    let part = crop(realm, &double, [1.0, 1.0, 4.0, 4.0]);
    let factors = call(realm, &part, "getScaleFactors", &[]).unwrap();
    assert_eq!(numbers(realm, &factors), [2.0]);
    assert_eq!(
        view(&call(realm, &part, "toBitmap", &[]).unwrap()).len(),
        256
    );
    let partial = object(
        realm,
        &[("x", Value::Number(0.0)), ("y", Value::Number(0.0))],
    );
    let thrown = call(realm, &image, "crop", &[partial]).unwrap_err();
    assert_eq!(
        message(realm, thrown),
        "TypeError: Error processing argument at index 0, conversion failure from "
    );
    // JPEG quality is clamped; only an integer is taken.
    let best = view(&call(realm, &image, "toJPEG", &[Value::Number(150.0)]).unwrap());
    let hundred = view(&call(realm, &image, "toJPEG", &[Value::Number(100.0)]).unwrap());
    assert_eq!(best, hundred);
    for (args, expected) in [
        (vec![], "TypeError: Insufficient number of arguments."),
        (
            vec![Value::str("80")],
            "TypeError: Error processing argument at index 0, conversion failure from 80",
        ),
        (
            vec![Value::Number(80.5)],
            "TypeError: Error processing argument at index 0, conversion failure from ",
        ),
    ] {
        let thrown = call(realm, &image, "toJPEG", &args).unwrap_err();
        assert_eq!(message(realm, thrown), expected);
    }
    // Only a picture at scale factor 1 is written as JPEG.
    let jpeg = call(realm, &double, "toJPEG", &[Value::Number(80.0)]).unwrap();
    assert!(view(&jpeg).is_empty());
    let png = view(&call(realm, &double, "toPNG", &[]).unwrap());
    assert_eq!(decode_png(&png).unwrap().width, 20);
    let url = call(realm, &image, "toDataURL", &[Value::Number(2.0)]).unwrap();
    assert!(
        ops::to_string(realm, &url)
            .unwrap()
            .to_rust_lossy()
            .starts_with("data:image/png;base64,iVBOR")
    );
    let warned = call(realm, &image, "getBitmap", &[]).unwrap();
    assert_eq!(view(&warned).len(), 4800);
    assert!(realm.electron_image_bitmap_warned);
}

#[test]
fn representations_are_added_from_buffers_and_urls() {
    let realm = &mut Realm::new();
    let module = Value::Object(module(realm));
    let image = call(realm, &module, "createEmpty", &[]).unwrap();
    let png = buffer(realm, bytes("src.png"));
    let options = object(
        realm,
        &[("scaleFactor", Value::Number(2.0)), ("buffer", png)],
    );
    call(realm, &image, "addRepresentation", &[options]).unwrap();
    assert!(matches!(
        call(realm, &image, "isEmpty", &[]),
        Ok(Value::Boolean(false))
    ));
    assert_eq!(size(realm, &image, &[]), (20.0, 15.0));
    let url = alloc::format!(
        "data:image/png;base64,{}",
        crate::encoding::base64_encode(&bytes("multi.png"), false)
    );
    let options = object(
        realm,
        &[
            ("scaleFactor", Value::Number(1.25)),
            ("dataURL", Value::str(&url)),
        ],
    );
    call(realm, &image, "addRepresentation", &[options]).unwrap();
    let raw = buffer(realm, vec![1; 24]);
    let options = object(
        realm,
        &[
            ("width", Value::Number(3.0)),
            ("height", Value::Number(2.0)),
            ("buffer", raw),
        ],
    );
    call(realm, &image, "addRepresentation", &[options]).unwrap();
    let factors = call(realm, &image, "getScaleFactors", &[]).unwrap();
    assert_eq!(numbers(realm, &factors), [2.0, 1.25, 1.0]);
    // Nothing to read adds nothing.
    let options = object(realm, &[("scaleFactor", Value::Number(3.0))]);
    call(realm, &image, "addRepresentation", &[options]).unwrap();
    let factors = call(realm, &image, "getScaleFactors", &[]).unwrap();
    assert_eq!(numbers(realm, &factors).len(), 3);
    assert_eq!(size(realm, &image, &[Value::Number(1.25)]), (6.0, 4.0));
    assert!(call(realm, &image, "addRepresentation", &[]).is_err());
}

#[test]
fn thumbnails_are_promised() {
    let realm = &mut Realm::new();
    let module = Value::Object(module(realm));
    let thumbnail = |realm: &mut Realm, args: &[Value]| {
        let promise = call(realm, &module, "createThumbnailFromPath", args).unwrap();
        let settled = alloc::rc::Rc::new(core::cell::RefCell::new(None));
        let (fulfilled, rejected) = (settled.clone(), settled.clone());
        let on_fulfilled = realm.function("", 1, false, move |realm, _, args| {
            let size = call(realm, &args[0], "getSize", &[])?;
            let width = ops::get_named(realm, &size, "width")?;
            *fulfilled.borrow_mut() = Some(ops::to_string(realm, &width)?.to_rust_lossy());
            Ok(Value::Undefined)
        });
        let on_rejected = realm.function("", 1, false, move |realm, _, args| {
            *rejected.borrow_mut() = Some(message(realm, Thrown(args[0].clone())));
            Ok(Value::Undefined)
        });
        call(
            realm,
            &promise,
            "then",
            &[Value::Object(on_fulfilled), Value::Object(on_rejected)],
        )
        .unwrap();
        realm.run_jobs().unwrap();
        settled.borrow().clone().unwrap()
    };
    let path = Value::str(&fixture("src.png"));
    let square = object(
        realm,
        &[
            ("width", Value::Number(16.0)),
            ("height", Value::Number(16.0)),
        ],
    );
    assert_eq!(thumbnail(realm, &[path.clone(), square.clone()]), "16");
    let missing = Value::str(&fixture("missing.png"));
    assert_eq!(
        thumbnail(realm, &[missing, square]),
        "Error: Failed to create IShellItem from the given path"
    );
    let flat = object(
        realm,
        &[
            ("width", Value::Number(0.0)),
            ("height", Value::Number(16.0)),
        ],
    );
    assert_eq!(
        thumbnail(realm, &[path.clone(), flat]),
        "Error: size must not be empty"
    );
    assert_eq!(
        thumbnail(realm, std::slice::from_ref(&path)),
        "TypeError: Error processing argument at index 1, conversion failure from undefined"
    );
    let half = object(realm, &[("width", Value::Number(16.0))]);
    assert_eq!(
        thumbnail(realm, &[path, half]),
        "TypeError: Error processing argument at index 1, conversion failure from "
    );
    // A path that is not a string gives nothing back.
    assert!(matches!(
        call(
            realm,
            &module,
            "createThumbnailFromPath",
            &[Value::Number(5.0)]
        ),
        Ok(Value::Undefined)
    ));
}

#[test]
fn clipboard_pictures_and_icon_pixels_are_straight_rgba() {
    let realm = &mut Realm::new();
    let image = Value::Object(clipboard_image(realm, Some(bytes("alpha.png")), None));
    let (width, height, rgba) = pixels(realm, &image).unwrap();
    assert_eq!((width, height), (12, 10));
    assert_eq!(rgba, decode_png(&bytes("alpha-electron.png")).unwrap().rgba);
    let from_path = pixels(realm, &Value::str(&fixture("src.png"))).unwrap();
    assert_eq!(from_path.0, 40);
    assert!(pixels(realm, &Value::Number(1.0)).is_none());
    let dib = dibv5(realm, &image).unwrap();
    assert_eq!(dib.len(), DIBV5_HEADER_SIZE + 480);
    let empty = Value::Object(clipboard_image(realm, None, Some(vec![0; 8])));
    assert!(dibv5(realm, &empty).is_err());
    assert!(dibv5(realm, &Value::Undefined).is_err());
}

#[test]
fn bitmaps_round_trip_through_png_as_premultiplied_pixels() {
    let realm = &mut Realm::new();
    let module = Value::Object(module(realm));
    // Measured in Electron 38: a colour above its alpha comes back clamped.
    let pixels = vec![
        10, 20, 30, 40, 200, 100, 50, 128, 0, 0, 0, 0, 255, 255, 255, 255,
    ];
    let size = object(
        realm,
        &[
            ("width", Value::Number(2.0)),
            ("height", Value::Number(2.0)),
        ],
    );
    let image = call(
        realm,
        &module,
        "createFromBitmap",
        &[buffer(realm, pixels), size],
    )
    .unwrap();
    let png = call(realm, &image, "toPNG", &[]).unwrap();
    let copy = call(realm, &module, "createFromBuffer", &[png]).unwrap();
    assert_eq!(
        view(&call(realm, &copy, "toBitmap", &[]).unwrap()),
        [
            10, 20, 30, 40, 128, 100, 50, 128, 0, 0, 0, 0, 255, 255, 255, 255
        ]
    );
}
