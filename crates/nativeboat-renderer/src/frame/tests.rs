use super::*;

#[test]
fn nearest_scroll_alignment_handles_large_and_small_extents() {
    for (start, size, delta) in [
        (-10.0, 200.0, 0.0),
        (-10.0, 20.0, -10.0),
        (-110.0, 200.0, -10.0),
        (90.0, 20.0, 10.0),
        (10.0, 200.0, 10.0),
        (10.0, 20.0, 0.0),
    ] {
        assert_eq!(
            scroll_delta(ScrollAlign::Nearest, start, size, 0.0, 100.0),
            delta
        );
    }
    assert_eq!(
        scroll_delta(ScrollAlign::Start, 20.0, 10.0, 5.0, 100.0),
        15.0
    );
    assert_eq!(
        scroll_delta(ScrollAlign::End, 20.0, 10.0, 5.0, 100.0),
        -75.0
    );
    assert_eq!(
        scroll_delta(ScrollAlign::Center, 20.0, 10.0, 5.0, 100.0),
        -30.0
    );
}
