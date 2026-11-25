use super::super::tests::{Store, fake, fake_contents, formats, handle_of};
use super::*;
use shimforge::{Session, mock};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::DataExchange::{CloseClipboard, OpenClipboard};
use windows_sys::core::BOOL;

fn set(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn dib(depth: u16, width: i32, height: i32, colors: &[[u8; 4]], pixels: &[u8]) -> Vec<u8> {
    let mut data = vec![0; 40];
    set(&mut data, 0, 40);
    set(&mut data, 4, width as u32);
    set(&mut data, 8, height as u32);
    set(&mut data, 12, 1 | (u32::from(depth) << 16));
    set(&mut data, 32, colors.len() as u32);
    for color in colors {
        data.extend_from_slice(color);
    }
    data.extend_from_slice(pixels);
    data
}

#[test]
fn dib_formats_decode_palette_padding_row_direction_masks_and_alpha() {
    let palette = [[0, 0, 255, 0], [255, 0, 0, 0]];
    for (depth, bits) in [(1, 0x40), (4, 0x01), (8, 0)] {
        let row = if depth == 8 {
            [0, 1, 0, 0]
        } else {
            [bits, 0, 0, 0]
        };
        assert_eq!(
            decode_dib(&dib(depth, 2, -1, &palette, &row)),
            Some(((2, 1), vec![255, 0, 0, 255, 0, 0, 255, 255]))
        );
    }
    let mut indexed = dib(1, 2, -1, &palette, &[0x40, 0, 0, 0]);
    set(&mut indexed, 32, 0); // The default full palette has two entries.
    assert!(decode_dib(&indexed).is_some());
    let red = vec![255, 0, 0, 255];
    assert_eq!(
        decode_dib(&dib(16, 1, 1, &[], &[0, 0x7c, 0, 0])),
        Some(((1, 1), red.clone()))
    );
    for height in [-2, 2] {
        let bytes = if height < 0 {
            [0, 0, 255, 99, 255, 0, 0, 99]
        } else {
            [255, 0, 0, 99, 0, 0, 255, 99]
        };
        assert_eq!(
            decode_dib(&dib(24, 1, height, &[], &bytes)),
            Some(((1, 2), vec![255, 0, 0, 255, 0, 0, 255, 255]))
        );
    }
    for (header, compression, depth, masks, pixel) in [
        (40, 3, 16, vec![0xf800, 0x07e0, 0x001f], vec![0, 0xf8, 0, 0]),
        (52, 3, 16, vec![0xf800, 0x07e0, 0x001f], vec![0, 0xf8, 0, 0]),
        (
            56,
            3,
            32,
            vec![0xff, 0xff00, 0xff0000, 0xff000000],
            vec![255, 0, 0, 255],
        ),
        (
            108,
            3,
            32,
            vec![0xff0000, 0xff00, 0xff, 0xff000000],
            vec![0, 0, 255, 255],
        ),
        (
            40,
            6,
            32,
            vec![0xff0000, 0xff00, 0xff, 0xff000000],
            vec![0, 0, 255, 255],
        ),
    ] {
        let mut data = dib(depth, 1, 1, &[], &[]);
        set(&mut data, 0, header);
        set(&mut data, 16, compression);
        data.resize((header as usize).max(40 + masks.len() * 4), 0);
        for (index, mask) in masks.into_iter().enumerate() {
            set(&mut data, 40 + index * 4, mask);
        }
        data.extend_from_slice(&pixel);
        assert_eq!(decode_dib(&data), Some(((1, 1), red.clone())));
    }
    let rgba = [255, 0, 0, 128, 0, 0, 0, 0, 0, 255, 0, 255, 0, 0, 255, 255];
    let mut data = encode_dib((2, 2), &rgba).unwrap();
    assert_eq!(&data[124..132], &[0, 255, 0, 255, 255, 0, 0, 255]);
    assert_eq!(decode_dib(&data), Some(((2, 2), rgba.to_vec())));
    let profile_offset = data.len();
    data.extend_from_slice(b"test-profile");
    set(&mut data, 112, profile_offset as u32);
    set(&mut data, 116, 12);
    assert_eq!(decode_dib(&data), Some(((2, 2), rgba.to_vec())));
    // A single invalid premultiplied color makes the whole 32-bit bitmap opaque.
    let data = dib(32, 2, -1, &[], &[0, 0, 255, 0, 10, 20, 30, 128]);
    assert_eq!(
        decode_dib(&data),
        Some(((2, 1), vec![255, 0, 0, 255, 30, 20, 10, 255]))
    );
    assert_eq!(component(7, 0), 255);
}

#[test]
fn malformed_dibs_are_rejected_before_pixels_are_read_or_allocated() {
    let good = dib(24, 1, 1, &[], &[0, 0, 255, 0]);
    for bytes in 0..good.len() {
        assert!(decode_dib(&good[..bytes]).is_none());
    }
    for (offset, value) in [
        (0, 12),
        (0, 125),
        (4, 0),
        (4, u32::MAX),
        (8, 0),
        (8, 0x80000000),
        (12, 2 | (24 << 16)),
        (12, 1 | (2 << 16)),
        (16, 1),
        (16, 3),
        (32, u32::MAX),
        (4, i32::MAX as u32),
    ] {
        let mut data = good.clone();
        set(&mut data, offset, value);
        assert!(decode_dib(&data).is_none(), "{offset}: {value}");
    }
    for (colors, bytes) in [(3, vec![0; 16]), (1, vec![0x80, 0, 0, 0])] {
        let mut data = dib(1, 1, 1, &[[0, 0, 0, 0]], &bytes);
        set(&mut data, 32, colors);
        assert!(decode_dib(&data).is_none());
    }
    for masks in [
        [0u32, 0xff00, 0xff],
        [0xff0000, 0xff00, 0xff00],
        [0x550000, 0xff00, 0xff],
        [0x10000, 0xff00, 0xff],
    ] {
        let mut data = dib(16, 1, 1, &[], &[]);
        set(&mut data, 16, 3);
        for mask in masks {
            data.extend_from_slice(&mask.to_le_bytes());
        }
        data.extend_from_slice(&[0; 4]);
        assert!(decode_dib(&data).is_none());
    }
    let mut data = vec![0; 52];
    set(&mut data, 0, 52);
    set(&mut data, 4, 1);
    set(&mut data, 8, 1);
    set(&mut data, 12, 1 | (32 << 16));
    set(&mut data, 16, 6);
    assert!(decode_dib(&data).is_none());
    for profile in [12, 9999] {
        let mut data = encode_dib((1, 1), &[0; 4]).unwrap();
        set(&mut data, 112, profile);
        set(&mut data, 116, 4);
        assert!(decode_dib(&data).is_none());
    }
    for (size, data) in [
        ((0, 0), vec![]),
        ((1, 1), vec![0; 3]),
        ((u32::MAX, 1), vec![]),
        ((1, u32::MAX), vec![]),
        ((i32::MAX as u32, i32::MAX as u32), vec![]),
    ] {
        assert!(encode_dib(size, &data).is_none());
    }
}

#[test]
fn native_image_clipboard_reads_png_first_and_falls_back_to_dib() {
    let store = Store::default();
    let mut session = Session::new();
    fake(&mut session, &store, true);
    formats(&mut session);
    let rgba = [255, 0, 0, 128, 0, 0, 0, 0, 0, 255, 0, 255, 0, 0, 255, 255];
    assert_eq!(
        image_from_pixels("clipboard-source", (2, 2), &rgba),
        Some((2.0, 2.0))
    );
    write_clipboard_image(
        Some("clipboard-source"),
        Some("image text"),
        Some("<b>image</b>"),
    )
    .unwrap();
    assert_eq!(super::super::read_text(), "image text");
    assert_eq!(super::super::read_html(), "<b>image</b>");
    assert_eq!(
        super::super::clipboard_formats(),
        ["text/plain", "text/html", "image/png"]
    );
    assert_eq!(read_clipboard_image("clipboard-png"), Some((2.0, 2.0)));
    assert_eq!(image_pixels("clipboard-png"), Some((2, 2, rgba.to_vec())));
    assert!(read_open(0xc004).unwrap().starts_with(PNG));
    store.lock().unwrap().remove(&0xc004);
    store
        .lock()
        .unwrap()
        .insert(u32::from(CF_DIB), handle_of(b"bad DIB") as usize);
    assert_eq!(read_clipboard_image("clipboard-v5"), Some((2.0, 2.0)));
    assert_eq!(image_pixels("clipboard-v5"), Some((2, 2, rgba.to_vec())));
    let memory = store.lock().unwrap().remove(&u32::from(CF_DIBV5)).unwrap();
    store.lock().unwrap().insert(u32::from(CF_DIB), memory);
    assert_eq!(
        super::super::clipboard_formats(),
        ["text/plain", "text/html", "image/png"]
    );
    assert_eq!(read_clipboard_image("clipboard-dib"), Some((2.0, 2.0)));
    store
        .lock()
        .unwrap()
        .insert(0xc004, handle_of(b"invalid PNG") as usize);
    assert_eq!(read_clipboard_image("clipboard-bad"), None);
    let mut bad_png = PNG.to_vec();
    bad_png.extend_from_slice(b"bad data");
    store
        .lock()
        .unwrap()
        .insert(0xc004, handle_of(&bad_png) as usize);
    assert_eq!(read_clipboard_image("clipboard-corrupt"), None);
    write_clipboard_image(None, Some(""), None).unwrap();
    assert_eq!(super::super::clipboard_formats(), ["text/plain"]);
    assert_eq!(read_clipboard_image("clipboard-empty"), None);
    assert!(write_clipboard_image(Some("missing-image"), None, None).is_err());
    drop(session);
    let mut session = Session::new();
    fake(&mut session, &store, false);
    formats(&mut session);
    assert_eq!(read_clipboard_image("clipboard-busy"), None);
    assert!(write_clipboard_image(None, None, None).is_err());
}

#[test]
fn image_snapshot_uses_one_open_and_png_precedes_native_bitmap_on_write() {
    let mut session = Session::new();
    formats(&mut session);
    mock!(
        session,
        image_pixels,
        fn(&str) -> Option<(u32, u32, Vec<u8>)>
    )
    .expect()
    .returning(|_| Some((1, 1, vec![255, 0, 0, 128])));
    mock!(session, image_encode, fn(&str, bool) -> Option<Vec<u8>>)
        .expect()
        .returning(|_, jpeg| {
            assert!(!jpeg);
            Some(PNG.to_vec())
        });
    mock!(
        session,
        super::super::write,
        fn(&[(u32, Vec<u8>)]) -> Result<(), Error>
    )
    .expect()
    .once()
    .returning(|entries| {
        assert_eq!(
            entries.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            [0xc004, u32::from(CF_DIBV5), 13, 0xc002]
        );
        assert_eq!(&entries[1].1[124..], [0, 0, 128, 128]);
        Ok(())
    });
    write_clipboard_image(Some("image"), Some("x"), Some("y")).unwrap();
    drop(session);
    let store = Store::default();
    store.lock().unwrap().insert(
        u32::from(CF_DIB),
        handle_of(&dib(24, 1, 1, &[], &[0, 0, 255, 0])) as usize,
    );
    let mut session = Session::new();
    fake_contents(&mut session, &store);
    formats(&mut session);
    mock!(
        session,
        OpenClipboard,
        unsafe extern "system" fn(HWND) -> BOOL
    )
    .expect()
    .once()
    .returning(|owner| {
        assert!(owner.is_null());
        1
    });
    mock!(session, CloseClipboard, unsafe extern "system" fn() -> BOOL)
        .expect()
        .once()
        .returning(|| 1);
    assert_eq!(read_clipboard_image("clipboard-snapshot"), Some((1.0, 1.0)));
}

#[test]
fn image_registration_and_encoding_failures_preserve_clipboard_contents() {
    let store = Store::default();
    let memory = handle_of(&dib(24, 1, 1, &[], &[0, 0, 255, 0])) as usize;
    store.lock().unwrap().insert(u32::from(CF_DIB), memory);
    let mut session = Session::new();
    fake(&mut session, &store, true);
    mock!(
        session,
        super::super::RegisterClipboardFormatW,
        unsafe extern "system" fn(*const u16) -> u32
    )
    .expect()
    .returning(|_| 0);
    assert_eq!(
        read_clipboard_image("clipboard-registration-fallback"),
        Some((1.0, 1.0))
    );
    assert_eq!(super::super::clipboard_formats(), ["image/png"]);
    assert!(
        write_clipboard_image(Some("clipboard-registration-fallback"), None, None)
            .unwrap_err()
            .message
            .starts_with("RegisterClipboardFormatW failed")
    );
    assert_eq!(store.lock().unwrap().get(&u32::from(CF_DIB)), Some(&memory));
    mock!(session, image_encode, fn(&str, bool) -> Option<Vec<u8>>)
        .expect()
        .returning(|_, _| None);
    assert_eq!(
        write_clipboard_image(Some("clipboard-registration-fallback"), None, None)
            .unwrap_err()
            .message,
        "Could not encode clipboard image"
    );
    assert_eq!(store.lock().unwrap().get(&u32::from(CF_DIB)), Some(&memory));
}
