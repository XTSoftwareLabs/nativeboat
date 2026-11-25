use super::*;

fn parse(text: &str, base: u32) -> f64 {
    integer(&text.encode_utf16().collect::<Vec<_>>(), base).unwrap()
}

#[test]
fn integers_round_once_at_ties_sticky_bits_carries_and_overflow() {
    for (text, base, expected) in [
        ("0", 2, 0.0f64),
        ("00000101", 2, 5.0),
        ("123", 4, 27.0),
        ("77", 8, 63.0),
        ("9007199254740993", 10, 9007199254740992.0),
        ("9007199254740995", 10, 9007199254740996.0),
        ("FFff", 16, 65535.0),
        ("vV", 32, 1023.0),
        ("Zz", 36, 1295.0),
    ] {
        assert_eq!(parse(text, base).to_bits(), expected.to_bits());
    }
    let power = f64::from_bits(1183u64 << 52); // 2^160.
    let tie = format!("1{}1{}", "0".repeat(52), "0".repeat(107));
    let sticky = format!("1{}1{}1", "0".repeat(52), "0".repeat(106));
    let odd = format!("1{}11{}", "0".repeat(51), "0".repeat(107));
    assert_eq!(parse(&tie, 2).to_bits(), power.to_bits());
    assert_eq!(parse(&sticky, 2).to_bits(), power.to_bits() + 1);
    assert_eq!(parse(&odd, 2).to_bits(), power.to_bits() + 2);
    assert_eq!(parse(&"1".repeat(161), 2), 2f64.powi(161));
    let largest = format!("{}{}", "1".repeat(53), "0".repeat(971));
    let below = format!("{}0{}", "1".repeat(53), "1".repeat(970));
    let midpoint = format!("{}{}", "1".repeat(54), "0".repeat(970));
    assert_eq!(parse(&largest, 2), f64::MAX);
    assert_eq!(parse(&below, 2), f64::MAX);
    assert_eq!(parse(&midpoint, 2), f64::INFINITY);
    assert_eq!(parse(&format!("1{}", "0".repeat(4096)), 2), f64::INFINITY);
    assert!(integer(&[], 10).is_none());
    for text in ["g", "١", "\u{1f600}"] {
        assert!(integer(&text.encode_utf16().collect::<Vec<_>>(), 16).is_none());
    }
    let invalid_after_overflow = format!("1{}x", "0".repeat(4096));
    assert!(
        integer(
            &invalid_after_overflow.encode_utf16().collect::<Vec<_>>(),
            2
        )
        .is_none()
    );
}
