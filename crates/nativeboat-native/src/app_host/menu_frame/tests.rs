use super::*;

/// The red channel of a backdrop pixel, premultiplied: over black, as
/// Windows shows a layered window's picture by itself.
fn red(picture: &(i32, i32, Vec<u8>), x: i32, y: i32) -> u8 {
    picture.2[((y * picture.0 + x) * 4 + 2) as usize]
}

fn near(actual: &[u8], expected: &[u8], tolerance: u8) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(a, e)| a.abs_diff(*e) <= tolerance)
}

#[test]
fn the_shadow_falls_as_electrons_does() {
    // Measured in Electron 38 at 125%: a 278×318 menu body's shadow, over
    // black, outward from its left edge, top edge and bottom edge, in the
    // middle of each.
    let scale = 1.25;
    let (side, above, _) = margins(scale);
    let picture = backdrop(278, 318, scale, [255, 255, 255]);
    let middle_x = side + 139;
    let left: Vec<u8> = (1..=14)
        .map(|step| red(&picture, side - step, above + 165))
        .collect();
    assert!(
        near(&left, &[8, 7, 6, 4, 4, 3, 3, 2, 2, 1, 1, 1, 1, 0], 1),
        "{left:?}"
    );
    let top: Vec<u8> = (1..=6)
        .map(|step| red(&picture, middle_x, above - step))
        .collect();
    assert!(near(&top, &[2, 1, 1, 1, 1, 0], 1), "{top:?}");
    let below: Vec<u8> = (1..=29)
        .map(|step| red(&picture, middle_x, above + 318 + step))
        .collect();
    let expected = [
        15, 14, 13, 12, 12, 12, 11, 11, 10, 10, 9, 9, 8, 7, 7, 6, 6, 5, 4, 4, 3, 3, 2, 2, 1, 1, 1,
        1, 0,
    ];
    assert!(near(&below, &expected, 1), "{below:?}");
    // The body is opaque white.
    let body = ((above + 100) * picture.0 + middle_x) as usize * 4;
    assert_eq!(picture.2[body..body + 4], [255, 255, 255, 255]);
}

#[test]
fn corners_are_rounded_with_antialiased_edges() {
    let scale = 1.25;
    let radius = RADIUS * scale;
    let body = (0.0, 0.0, 278.0, 318.0);
    assert_eq!(coverage(0.0, 0.0, body, radius), 0.0);
    assert_eq!(coverage(20.0, 0.0, body, radius), 1.0);
    let edge = coverage(4.0, 4.0, body, radius);
    assert!(edge > 0.0 && edge < 1.0, "{edge}");
    // The window keeps only whole pixels: corner rows are narrower.
    let spans = region_spans(278, 318, scale);
    assert_eq!(spans.len(), 318);
    let (row, first, last) = spans[0];
    assert!(row == 0 && first > 5 && last < 273, "{first} {last}");
    assert_eq!(spans[159], (159, 0, 278));
    assert_eq!(spans[317].1, first);
    // Margins reach past the wider, lower layer.
    let (side, above, below) = margins(scale);
    assert!(
        below > side && side > above && above >= 0,
        "{side} {above} {below}"
    );
}
