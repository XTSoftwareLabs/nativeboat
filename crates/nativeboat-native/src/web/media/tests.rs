use super::*;

fn fixture(name: &str) -> String {
    alloc::format!(
        "{}\\src\\web\\media\\{name}",
        env!("CARGO_MANIFEST_DIR").replace('/', "\\")
    )
}

/// The RGBA pixel of a frame at a point.
fn at(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
    let start = ((y * frame.width + x) * 4) as usize;
    frame.pixels[start..start + 4].try_into().unwrap()
}

fn reddish([red, green, blue, alpha]: [u8; 4]) -> bool {
    red > 200 && green < 80 && blue < 80 && alpha == 255
}

fn bluish([red, green, blue, alpha]: [u8; 4]) -> bool {
    red < 80 && green < 80 && blue > 200 && alpha == 255
}

/// Whether two colours are within the few levels rounding leaves.
fn near(colour: [u8; 3], expected: [u8; 3]) -> bool {
    colour
        .iter()
        .zip(expected)
        .all(|(channel, expected)| channel.abs_diff(expected) <= 3)
}

#[test]
fn videos_give_their_duration_and_first_frame() {
    // 64×36 red H.264, one second long: decoders pad it to 48 lines, which
    // are cut off.
    let media = read(&MediaSource::Path(fixture("tiny.mp4"))).unwrap();
    assert!((media.duration - 1.0).abs() < 0.01, "{}", media.duration);
    let frame = media.frame.unwrap();
    assert_eq!((frame.width, frame.height), (64, 36));
    assert_eq!(frame.pixels.len(), 64 * 36 * 4);
    assert!(
        reddish(at(&frame, 0, 0)) && reddish(at(&frame, 63, 35)),
        "{:?}",
        at(&frame, 0, 0)
    );
    // Untagged and under 720 lines, its red has SMPTE 170M primaries, which
    // Electron 38 shows as (248, 36, 0).
    let [red, green, blue, _] = at(&frame, 32, 18);
    assert!(
        near([red, green, blue], [248, 36, 0]),
        "{red} {green} {blue}"
    );
    // From memory, as a `data:` URL gives it.
    let bytes = std::fs::read(fixture("av.mp4")).unwrap();
    let media = read(&MediaSource::Bytes(bytes)).unwrap();
    assert!((media.duration - 1.5).abs() < 0.05, "{}", media.duration);
    let frame = media.frame.unwrap();
    assert_eq!((frame.width, frame.height), (50, 40));
    let [red, green, blue, _] = at(&frame, 25, 20);
    assert!(green > 200 && red < 80 && blue < 80);
}

#[test]
fn rotated_videos_are_turned_upright_as_chromium_shows_them() {
    // Red over blue, recorded turned: Chromium shows a 90° display
    // rotation with red on the left, and 270° with red on the right.
    for (name, red_left) in [("rot90.mp4", true), ("rot270.mp4", false)] {
        let frame = read(&MediaSource::Path(fixture(name)))
            .unwrap()
            .frame
            .unwrap();
        assert_eq!((frame.width, frame.height), (36, 64), "{name}");
        let (left, right) = (at(&frame, 2, 32), at(&frame, 33, 32));
        if red_left {
            assert!(reddish(left) && bluish(right), "{name}: {left:?} {right:?}");
        } else {
            assert!(bluish(left) && reddish(right), "{name}: {left:?} {right:?}");
        }
    }
}

#[test]
fn audio_has_a_duration_and_no_frame() {
    let media = read(&MediaSource::Path(fixture("tone.mp3"))).unwrap();
    assert!((media.duration - 1.0).abs() < 0.1, "{}", media.duration);
    assert_eq!(media.frame, None);
}

#[test]
fn unreadable_media_reads_as_nothing() {
    assert_eq!(read(&MediaSource::Path(fixture("missing.mp4"))), None);
    assert_eq!(read(&MediaSource::Bytes(b"not a video".to_vec())), None);
    assert_eq!(read(&MediaSource::Bytes(Vec::new())), None);
}

#[test]
fn colours_are_shown_as_chromium_shows_them() {
    // Pure red as untagged video stores it: BT.601 in studio range.
    let (y, u, v) = (81, 90, 240);
    // Measured in Electron 38: under 720 lines untagged video has SMPTE 170M
    // primaries, which Chromium maps to sRGB's, and from 720 lines it is
    // BT.709, so the same samples are a different red.
    let standard = Colour::of(None, None, None, 718);
    assert!(
        near(standard.rgb(y, u, v), [248, 36, 0]),
        "{:?}",
        standard.rgb(y, u, v)
    );
    let high = Colour::of(None, None, None, 720);
    assert!(
        near(high.rgb(y, u, v), [255, 24, 0]),
        "{:?}",
        high.rgb(y, u, v)
    );
    // Tagged BT.709 red, in studio and full range.
    let studio = Colour::of(Some(1), Some(2), Some(2), 180);
    assert!(near(studio.rgb(63, 102, 240), [255, 0, 0]));
    let full = Colour::of(Some(1), Some(1), Some(2), 180);
    assert!(near(full.rgb(54, 99, 255), [255, 0, 0]));
    // Studio range greys stretch to the full range, with no other curve,
    // whatever the primaries.
    assert_eq!(studio.rgb(16, 128, 128), [0, 0, 0]);
    assert_eq!(studio.rgb(235, 128, 128), [255, 255, 255]);
    assert_eq!(studio.rgb(128, 128, 128), [130, 130, 130]);
    assert!(near(standard.rgb(128, 128, 128), [130, 130, 130]));
    // Tagged SMPTE 170M primaries are mapped whatever the size.
    assert!(Colour::of(Some(2), Some(2), Some(5), 1080).smpte_170m);
    assert!(!Colour::of(Some(2), Some(2), Some(2), 180).smpte_170m);
}

#[test]
fn pictures_are_read_upright_from_their_planes() {
    // A 2×2 NV12 picture of greys, rows 4 bytes apart: 10, 20 over 30, 40,
    // then one neutral U and V pair for all four.
    let bytes = [10, 20, 9, 9, 30, 40, 9, 9, 128, 128, 9, 9];
    let colour = Colour::of(Some(1), Some(1), Some(2), 2);
    let grey = |level: u8| [level, level, level, 255];
    let read =
        |shown, rotation| upright(nv12(&bytes, (2, 2), 4, &colour).unwrap(), shown, rotation);
    let all = (0, 0, 2, 2);
    assert_eq!(
        read(all, 0).pixels,
        [grey(10), grey(20), grey(30), grey(40)].concat()
    );
    // Turned clockwise.
    assert_eq!(
        read(all, 90).pixels,
        [grey(30), grey(10), grey(40), grey(20)].concat()
    );
    assert_eq!(
        read(all, 180).pixels,
        [grey(40), grey(30), grey(20), grey(10)].concat()
    );
    // The shown part only.
    let corner = read((1, 1, 1, 1), 0);
    assert_eq!(
        (corner.width, corner.height, corner.pixels),
        (1, 1, grey(40).to_vec())
    );
    // Too few bytes for the planes, or rows narrower than the picture.
    assert!(nv12(&bytes[..9], (2, 2), 4, &colour).is_none());
    assert!(nv12(&bytes, (8, 2), 4, &colour).is_none());
}
