use super::*;

/// The grey a black shape on white shows at a pixel.
fn grey(shape: &Shape, x: i32, y: i32) -> u8 {
    (255.0 * (1.0 - shape.coverage(x, y)) + 0.5) as u8
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> [i32; 4] {
    [left, top, right, bottom]
}

#[test]
fn corners_are_covered_as_chromium_covers_them() {
    // Measured in Electron 38: a black 60×40 box at 10, 10 with 10px
    // corners, its top row and left column.
    let shape = Shape {
        rect: rect(10, 10, 70, 50),
        radii: [(10.0, 10.0); 4],
        inner: None,
    };
    let row: Vec<u8> = (10..21).map(|x| grey(&shape, x, 10)).collect();
    assert_eq!(row, [255, 255, 255, 255, 255, 255, 159, 82, 30, 3, 0]);
    let column: Vec<u8> = (10..21).map(|y| grey(&shape, 10, y)).collect();
    assert_eq!(column, [255, 255, 255, 255, 255, 255, 159, 82, 30, 3, 0]);
    assert_eq!(grey(&shape, 13, 12), 108);
    // The bottom right corner mirrors it; inside, and outside, are whole.
    assert_eq!(grey(&shape, 69, 43), 159);
    assert_eq!(grey(&shape, 40, 30), 0);
    assert_eq!(grey(&shape, 9, 30), 255);
    // A small radius.
    let small = Shape {
        rect: rect(250, 10, 310, 50),
        radii: [(2.5, 2.5); 4],
        inner: None,
    };
    let corner: Vec<u8> = [(250, 10), (251, 10), (250, 11), (251, 11)]
        .into_iter()
        .map(|(x, y)| grey(&small, x, y))
        .collect();
    assert_eq!(corner, [211, 60, 60, 0]);
}

#[test]
fn elliptical_corners_are_covered_as_chromium_covers_them() {
    // Measured in Electron 38: black 60×40 boxes at 90, 10 with 20px / 10px
    // corners and at 170, 130 with 50% ones, within three levels.
    let near = |actual: Vec<u8>, expected: &[u8]| {
        assert!(
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected)
                    .all(|(a, e)| a.abs_diff(*e) <= 3),
            "{actual:?} vs {expected:?}"
        );
    };
    let wide = Shape {
        rect: rect(90, 10, 150, 50),
        radii: [(20.0, 10.0); 4],
        inner: None,
    };
    near(
        (101..111).map(|x| grey(&wide, x, 10)).collect(),
        &[222, 179, 137, 100, 69, 40, 20, 1, 0, 0],
    );
    near(
        (16..21).map(|y| grey(&wide, 90, y)).collect(),
        &[248, 149, 68, 24, 0],
    );
    near(
        (129..140).map(|x| grey(&wide, x, 49)).collect(),
        &[0, 0, 1, 20, 40, 69, 100, 137, 179, 222, 255],
    );
    let oval = Shape {
        rect: rect(170, 130, 230, 170),
        radii: [(30.0, 20.0); 4],
        inner: None,
    };
    near(
        (190..201).map(|x| grey(&oval, x, 130)).collect(),
        &[239, 195, 157, 121, 89, 62, 37, 21, 4, 0, 0],
    );
    // Down its left side, past the middle where the corners meet.
    near(
        (144..156).map(|y| grey(&oval, 170, y)).collect(),
        &[247, 176, 118, 70, 26, 8, 8, 26, 70, 118, 176, 247],
    );
    // Its four corner rectangles tile the box.
    let tiles = oval.tiles();
    let area: i32 = tiles.iter().map(|t| (t[2] - t[0]) * (t[3] - t[1])).sum();
    assert_eq!((tiles.len(), area), (4, 60 * 40));
}

#[test]
fn rings_cover_what_their_outer_edge_covers_and_their_inner_edge_does_not() {
    // Measured in Electron 38: a 3px black border with 10px corners.
    let ring = Shape {
        rect: rect(10, 70, 70, 110),
        radii: [(10.0, 10.0); 4],
        inner: Some(([13.0, 73.0, 67.0, 107.0], [(7.0, 7.0); 4])),
    };
    let row: Vec<u8> = (10..22).map(|x| grey(&ring, x, 73)).collect();
    assert_eq!(row, [255, 255, 108, 0, 0, 0, 30, 137, 211, 250, 255, 255]);
    let row: Vec<u8> = (10..22).map(|x| grey(&ring, x, 76)).collect();
    assert_eq!(row, [159, 0, 0, 30, 250, 255, 255, 255, 255, 255, 255, 255]);
    // A square inner corner, as a border wider than its radius leaves.
    let thick = Shape {
        rect: rect(0, 0, 20, 20),
        radii: [(4.0, 4.0); 4],
        inner: Some(([6.0, 6.0, 14.0, 14.0], [(0.0, 0.0); 4])),
    };
    assert_eq!(grey(&thick, 6, 6), 255);
    assert_eq!(grey(&thick, 5, 6), 0);
}

#[test]
fn only_corner_squares_are_blended_unless_they_meet() {
    let shape = Shape {
        rect: rect(0, 0, 30, 20),
        radii: [(4.0, 4.0), (2.5, 2.5), (0.0, 0.0), (6.0, 6.0)],
        inner: None,
    };
    assert_eq!(
        shape.tiles(),
        [rect(0, 0, 4, 4), rect(27, 0, 30, 3), rect(0, 14, 6, 20)]
    );
    // Squares that overlap make one tile of the whole box.
    let pill = Shape {
        rect: rect(0, 0, 30, 8),
        radii: [(5.0, 5.0); 4],
        inner: None,
    };
    assert_eq!(pill.tiles(), [rect(0, 0, 30, 8)]);
    let square = Shape {
        rect: rect(0, 0, 30, 10),
        radii: [(0.0, 0.0); 4],
        inner: None,
    };
    assert!(square.tiles().is_empty());
}

#[test]
fn painted_corners_are_mixed_with_what_was_beneath() {
    // An 8×8 white picture with a black 6×6 box painted at 1, 1.
    let mut pixels = [0x00ff_ffff_u32; 64];
    let bitmap = BITMAP {
        bmBitsPixel: 32,
        bmWidth: 8,
        bmWidthBytes: 32,
        bmHeight: 8,
        bmBits: pixels.as_mut_ptr().cast(),
        ..BITMAP::default()
    };
    // SAFETY: the pixels outlive the groups and nothing draws meanwhile.
    let mut coverage = unsafe { Coverage::new(bitmap) };
    let shape = Shape {
        rect: rect(1, 1, 7, 7),
        radii: [(3.0, 3.0); 4],
        inner: None,
    };
    coverage.push(shape);
    // A shape partly outside the picture blends only what is inside it.
    coverage.push(Shape {
        rect: rect(-2, -2, 2, 2),
        radii: [(2.0, 2.0); 4],
        inner: None,
    });
    for y in 1..7 {
        for x in 1..7 {
            pixels[y * 8 + x] = 0x7700_0000;
        }
    }
    coverage.pop();
    coverage.pop();
    assert!(coverage.is_empty());
    // The corner pixel is mostly white, the middle black, and the unused
    // fourth byte is the painted one's.
    let channel = |pixel: u32| pixel & 255;
    assert_eq!(channel(pixels[8 + 1]), u32::from(grey(&shape, 1, 1)));
    assert_eq!(pixels[3 * 8 + 3], 0x7700_0000);
    assert_eq!(pixels[8 + 1] >> 24, 0x77);
    assert_eq!(pixels[0], 0x00ff_ffff);
    // Nothing open, nothing done; an unusable bitmap has no pixels.
    coverage.pop();
    // SAFETY: an unusable bitmap is never read.
    let mut empty = unsafe { Coverage::new(BITMAP::default()) };
    empty.push(shape);
    empty.pop();
    assert_eq!(mix(5, 9, 1.0), 5);
}
