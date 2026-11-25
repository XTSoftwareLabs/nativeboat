use super::*;

#[test]
fn notification_icons_are_centered_in_squares_and_scaled_down() {
    // A 4x2 image: opaque red on the left half, transparent on the right.
    let mut rgba = Vec::new();
    for _ in 0..2 {
        for x in 0..4 {
            rgba.extend_from_slice(if x < 2 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 0]
            });
        }
    }
    // Small images keep their pixels, centered on transparent rows.
    let (side, square) = square_icon(4, 2, &rgba, 64).unwrap();
    assert_eq!(side, 4);
    assert_eq!(&square[..16], &[0; 16]);
    assert_eq!(&square[16..20], &[255, 0, 0, 255]);
    assert_eq!(&square[28..32], &[0, 0, 0, 0]);
    // Larger ones are averaged down: transparent pixels add no color.
    let (side, square) = square_icon(4, 2, &rgba, 1).unwrap();
    assert_eq!(side, 1);
    assert_eq!(square, [255, 0, 0, 127]);
    let (side, square) = square_icon(4, 2, &rgba, 2).unwrap();
    assert_eq!(side, 2);
    assert_eq!(square, [255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(square_icon(2, 2, &rgba, 64).is_none());
    assert!(square_icon(0, 2, &[], 64).is_none());
}
