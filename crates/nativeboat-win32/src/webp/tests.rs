use super::*;

/// FNV-1a over decoded pixels. The fixtures' digests are of the pixels
/// libwebp 1.6.0 decodes them to; `scripts/generate-webp-*.py` write them
/// and print their digests.
fn digest(pixels: &[u8]) -> u64 {
    pixels.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A fixture's name, file, size and digest.
type Fixture = (&'static str, &'static [u8], (u32, u32), u64);

const FIXTURES: [Fixture; 12] = [
    // Lossy, odd-sized, in segments with the normal loop filter.
    (
        "lossy",
        include_bytes!("fixtures/lossy.webp"),
        (45, 31),
        0x9aed_c84e_6108_5b6b,
    ),
    // Coarsely quantized, with strong filtering and whole-block modes.
    (
        "lossy-low",
        include_bytes!("fixtures/lossy-low.webp"),
        (64, 40),
        0x6ef1_77c7_5a43_9336,
    ),
    // Written by a test encoder with what libwebp's encoder never writes:
    // the simple loop filter, filter deltas, a segment map with values
    // relative to the frame's, quantizer deltas, skipped macroblocks and
    // four token partitions.
    (
        "lossy-simple",
        include_bytes!("fixtures/lossy-simple.webp"),
        (70, 66),
        0x16db_d894_4c75_49e3,
    ),
    // The normal loop filter without segments, at high sharpness, in eight
    // token partitions.
    (
        "lossy-partitions",
        include_bytes!("fixtures/lossy-partitions.webp"),
        (40, 72),
        0xcc20_6953_0cf5_8f96,
    ),
    // Segments without a map, so all in the first, and no loop filter.
    (
        "lossy-plain",
        include_bytes!("fixtures/lossy-plain.webp"),
        (35, 20),
        0x3754_ff41_13ae_d108,
    ),
    // Lossy with losslessly compressed, filtered alpha.
    (
        "lossy-alpha",
        include_bytes!("fixtures/lossy-alpha.webp"),
        (33, 21),
        0xa11a_8044_e0f9_01f0,
    ),
    // Alpha reduced to fewer levels, which libwebp leaves as decoded.
    (
        "lossy-alpha-quantized",
        include_bytes!("fixtures/lossy-alpha-quantized.webp"),
        (30, 18),
        0xb576_0c09_a630_63df,
    ),
    // Lossless with the predictor and cross-color transforms and a color
    // cache.
    (
        "lossless",
        include_bytes!("fixtures/lossless.webp"),
        (40, 24),
        0x4c6d_127c_a82c_9ab8,
    ),
    // A few colors, indexed and packed several to a pixel.
    (
        "lossless-flat",
        include_bytes!("fixtures/lossless-flat.webp"),
        (23, 17),
        0x8648_1141_8fe2_d26f,
    ),
    // Written by a test encoder: meta prefix codes with a group no tile
    // uses, every predictor mode, subtract green, code lengths with repeats
    // and a symbol limit, simple codes, backward references and the cache.
    (
        "lossless-meta",
        include_bytes!("fixtures/lossless-meta.webp"),
        (52, 37),
        0x133e_4362_4990_5531,
    ),
    (
        "lossless-fast",
        include_bytes!("fixtures/lossless-fast.webp"),
        (29, 13),
        0x9be8_c027_1819_bc8c,
    ),
    // An animation shows its first frame.
    (
        "animated",
        include_bytes!("fixtures/animated.webp"),
        (24, 16),
        0xbd0d_db14_d33b_4992,
    ),
];

#[test]
fn webp_images_decode_to_the_pixels_libwebp_gives() {
    for (name, bytes, size, expected) in FIXTURES {
        let (decoded, pixels) = decode(bytes).unwrap_or_else(|| panic!("{name} decodes"));
        assert_eq!(decoded, size, "{name}");
        assert_eq!(
            pixels.len(),
            size.0 as usize * size.1 as usize * 4,
            "{name}"
        );
        assert_eq!(digest(&pixels), expected, "{name}");
    }
}

#[test]
fn broken_webp_images_do_not_decode() {
    for (name, bytes, ..) in FIXTURES {
        assert!(is_webp(bytes), "{name}");
        // Cut short, the bitstream runs out.
        assert_eq!(decode(&bytes[..bytes.len() / 3]), None, "{name}");
        assert_eq!(decode(&bytes[..16]), None, "{name}");
        // A broken signature or header.
        let mut broken = bytes.to_vec();
        let bitstream = (12..bytes.len())
            .find(|&at| matches!(&bytes[at..at + 4], b"VP8 " | b"VP8L"))
            .unwrap();
        broken[bitstream + 8..bitstream + 14].fill(0xff);
        assert_eq!(decode(&broken), None, "{name}");
    }
    assert!(!is_webp(b"RIFF\0\0\0\0WAVEfmt "));
    assert_eq!(decode(b"not an image at all"), None);
    // An animation's canvas too large to hold.
    let mut animated = FIXTURES[11].1.to_vec();
    animated[24..30].fill(0xff);
    assert_eq!(decode(&animated), None);
    // An extended file without a bitstream, and an animation without frames.
    let mut extended = b"RIFF\x1a\0\0\0WEBPVP8X\x0a\0\0\0".to_vec();
    extended.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 1, 0, 0]);
    assert_eq!(decode(&extended), None);
    extended[20] = 0x02;
    assert_eq!(decode(&extended), None);
}

#[test]
fn color_profiles_are_ignored() {
    // An ICCP chunk after the extended header, before the alpha and the
    // bitstream.
    let (name, bytes, size, expected) = FIXTURES[5];
    assert_eq!(name, "lossy-alpha");
    let mut profiled = bytes[..30].to_vec();
    profiled.extend_from_slice(b"ICCP\x03\0\0\0abc\0");
    profiled.extend_from_slice(&bytes[30..]);
    let riff = (profiled.len() - 8) as u32;
    profiled[4..8].copy_from_slice(&riff.to_le_bytes());
    let (decoded, pixels) = decode(&profiled).unwrap();
    assert_eq!((decoded, digest(&pixels)), (size, expected));
}

/// A fixture's VP8 frame.
fn frame(index: usize) -> Vec<u8> {
    let bytes = FIXTURES[index].1;
    let at = (12..bytes.len())
        .find(|&at| &bytes[at..at + 4] == b"VP8 ")
        .unwrap();
    bytes[at + 8..].to_vec()
}

#[test]
fn broken_lossy_frames_do_not_decode() {
    // No width, or too large to hold.
    let mut sized = frame(2);
    sized[6..8].fill(0);
    assert_eq!(lossy::decode(&sized, None), None);
    sized[6..10].fill(0xff);
    assert_eq!(lossy::decode(&sized, None), None);
    // The first partition, with the frame header and the modes, cut short
    // in its segment header, its filter header or its modes. Cut in a frame
    // of one token partition, the rest is still a partition.
    for index in [0, 2] {
        let whole = frame(index);
        let first_size = (u32::from_le_bytes([whole[0], whole[1], whole[2], 0]) >> 5) as usize;
        for size in 0..first_size / 2 {
            let mut cut = whole.clone();
            let tag = u32::from(whole[0] & 0x1f) | (size as u32) << 5;
            cut[..3].copy_from_slice(&tag.to_le_bytes()[..3]);
            assert_eq!(lossy::decode(&cut, None), None, "{index} {size}");
        }
    }
    // The sizes of eight token partitions cut short.
    let partitioned = frame(3);
    let first_size =
        (u32::from_le_bytes([partitioned[0], partitioned[1], partitioned[2], 0]) >> 5) as usize;
    assert_eq!(
        lossy::decode(&partitioned[..10 + first_size + 10], None),
        None
    );
}

/// Packs fields of the given numbers of bits least significant bit first, as
/// VP8L stores them.
fn pack(fields: &[(u32, u32)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut position = 0;
    for &(value, count) in fields {
        for bit in 0..count {
            if position % 8 == 0 {
                bytes.push(0);
            }
            bytes[position / 8] |= (((value >> bit) & 1) as u8) << (position % 8);
            position += 1;
        }
    }
    bytes
}

/// A VP8L header for an image of this size, without alpha.
fn vp8l(width: u32, height: u32) -> Vec<(u32, u32)> {
    vec![(0x2f, 8), (width - 1, 14), (height - 1, 14), (0, 1), (0, 3)]
}

/// A simple prefix code of one symbol, 0 or 1.
const ONE_SYMBOL: [(u32, u32); 4] = [(1, 1), (0, 1), (0, 1), (0, 1)];

/// A green code of code lengths themselves coded by these lengths of the
/// code length code's first four symbols, 17, 18, 0 and 1.
fn coded_lengths(lengths: [u32; 4]) -> Vec<(u32, u32)> {
    let mut fields = vec![(0, 1), (0, 4)];
    fields.extend(lengths.map(|length| (length, 3)));
    fields
}

#[test]
fn broken_lossless_streams_do_not_decode() {
    let lossless = |fields: &[(u32, u32)]| lossless::decode(&pack(fields));
    let with = |header: Vec<(u32, u32)>, rest: &[(u32, u32)]| [header, rest.to_vec()].concat();
    // An unknown version, or too large to hold.
    let mut version = vp8l(1, 1);
    version[4] = (1, 3);
    assert_eq!(lossless(&version), None);
    assert_eq!(lossless(&vp8l(16384, 16384)), None);
    // A transform twice.
    assert_eq!(
        lossless(&with(vp8l(1, 1), &[(1, 1), (2, 2), (1, 1), (2, 2)])),
        None
    );
    // Color caches of no bits and of more than eleven.
    assert_eq!(lossless(&with(vp8l(1, 1), &[(0, 1), (1, 1), (0, 4)])), None);
    assert_eq!(
        lossless(&with(vp8l(1, 1), &[(0, 1), (1, 1), (12, 4)])),
        None
    );
    // No transform, no cache, no meta codes, then a green code whose code
    // length code has only the symbol 0, so all its lengths are 0.
    let plain = |width| with(vp8l(width, 1), &[(0, 1), (0, 1), (0, 1)]);
    let codes = plain(1);
    assert_eq!(
        lossless(&with(
            codes.clone(),
            &[coded_lengths([0, 0, 1, 0]), vec![(0, 1)]].concat()
        )),
        None
    );
    // Code length codes that oversubscribe or leave codes unused.
    assert_eq!(
        lossless(&with(codes.clone(), &coded_lengths([1, 1, 1, 0]))),
        None
    );
    assert_eq!(
        lossless(&with(codes.clone(), &coded_lengths([1, 0, 2, 0]))),
        None
    );
    // More symbols than the alphabet has: 2 + 65535 of 280.
    let limited = [
        coded_lengths([0, 0, 1, 1]),
        vec![(1, 1), (7, 3), (0xffff, 16)],
    ]
    .concat();
    assert_eq!(lossless(&with(codes.clone(), &limited)), None);
    // Zeros repeated past the alphabet: 18, coded "1", three times 138.
    let mut repeated = [coded_lengths([0, 1, 1, 0]), vec![(0, 1)]].concat();
    repeated.extend([(1, 1), (127, 7)].repeat(3));
    assert_eq!(lossless(&with(codes.clone(), &repeated)), None);
    // A backward reference before the first pixel: green codes 0 and 256,
    // both of one bit, from code lengths "1", then 255 "0"s, then "1".
    let mut referenced = [coded_lengths([0, 0, 1, 1]), vec![(1, 1), (3, 3), (255, 8)]].concat();
    referenced.push((1, 1));
    referenced.extend([(0, 1)].repeat(255));
    referenced.push((1, 1));
    for _ in 0..4 {
        referenced.extend(ONE_SYMBOL);
    }
    // Length 1 at distance 2.
    referenced.push((1, 1));
    assert_eq!(lossless(&with(plain(2), &referenced)), None);
    // A stream that ends with its predictor transform's modes, before the
    // image's own codes: 40 header bits, 6 for the transform, then for 5x1
    // modes no cache, a two-symbol green code of 12 bits and four others
    // of 4, and a bit a mode.
    let mut ended = with(vp8l(17, 1), &[(1, 1), (0, 2), (0, 3), (0, 1)]);
    ended.extend([(1, 1), (1, 1), (0, 1), (0, 1), (1, 8)]);
    for _ in 0..4 {
        ended.extend(ONE_SYMBOL);
    }
    ended.extend([(1, 1)].repeat(5));
    assert_eq!(pack(&ended).len(), 10);
    assert_eq!(lossless(&ended), None);
}

#[test]
fn raw_alpha_planes_undo_their_row_filters() {
    let deltas = [10, 5, 250, 0, 3, 1];
    let plane = |filter: u8| alpha_plane(&[[filter << 2].as_slice(), &deltas].concat(), (3, 2));
    // Horizontal: from the left, the first column from above.
    assert_eq!(plane(1), Some(vec![10, 15, 9, 10, 13, 14]));
    // Vertical: from above, the first row from the left.
    assert_eq!(plane(2), Some(vec![10, 15, 9, 10, 18, 10]));
    // Gradient: left plus above less above-left, clamped.
    assert_eq!(plane(3), Some(vec![10, 15, 9, 10, 18, 13]));
    assert_eq!(plane(0), Some(deltas.to_vec()));
    // Too short, an unknown method or reserved bits.
    assert_eq!(alpha_plane(&[0, 1, 2], (3, 2)), None);
    assert_eq!(alpha_plane(&[2, 1, 2, 3, 4, 5, 6], (3, 2)), None);
    assert_eq!(alpha_plane(&[0x40, 1, 2, 3, 4, 5, 6], (3, 2)), None);
    assert_eq!(alpha_plane(&[0], (3, 2)), None);
}

/// Compares decoding with libwebp's over the files `WEBP_CORPUS` names: a
/// file whose lines each hold a WebP file's path and the path of the RGBA
/// libwebp decoded it to, separated by a tab.
#[test]
#[ignore]
fn corpus_decodes_as_libwebp_does() {
    let list = std::fs::read_to_string(std::env::var("WEBP_CORPUS").unwrap()).unwrap();
    let mut failures = Vec::new();
    for line in list.lines() {
        let (webp, rgba) = line.split_once('\t').unwrap();
        let expected = std::fs::read(rgba).unwrap();
        let started = std::time::Instant::now();
        let decoded = decode(&std::fs::read(webp).unwrap());
        let elapsed = started.elapsed();
        let different = decoded.as_ref().map(|(_, pixels)| {
            pixels.iter().zip(&expected).filter(|(a, b)| a != b).count()
                + pixels.len().abs_diff(expected.len())
        });
        std::println!("{webp}: {different:?} bytes differ, {elapsed:?}");
        if different != Some(0) {
            failures.push(webp);
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
