use super::*;

const LIMITS: Limits = Limits {
    time: 500_000,
    width: 4,
    height: 4,
};

fn at(time: u64, point: (i32, i32)) -> Press {
    Press {
        time,
        point,
        button: 0,
        modifiers: (false, false, false, false),
        count: 1,
    }
}

#[test]
fn presses_repeat_within_the_double_click_time_and_rectangle() {
    let first = at(1_000, (10, 10));
    assert_eq!(repeat(None, &first, LIMITS), 1);
    let second = Press {
        count: 2,
        ..at(400_000, (12, 8))
    };
    assert_eq!(repeat(Some(&first), &second, LIMITS), 2);
    // A third press counts on, as a triple click.
    assert_eq!(repeat(Some(&second), &at(800_000, (11, 9)), LIMITS), 3);
    // Too late, too far, another button or other modifiers start again.
    assert_eq!(repeat(Some(&first), &at(501_001, (10, 10)), LIMITS), 1);
    assert_eq!(repeat(Some(&first), &at(2_000, (13, 10)), LIMITS), 1);
    assert_eq!(repeat(Some(&first), &at(2_000, (10, 7)), LIMITS), 1);
    let right = Press {
        button: 2,
        ..at(2_000, (10, 10))
    };
    assert_eq!(repeat(Some(&first), &right, LIMITS), 1);
    let shifted = Press {
        modifiers: (true, false, false, false),
        ..at(2_000, (10, 10))
    };
    assert_eq!(repeat(Some(&first), &shifted, LIMITS), 1);
    // The same moment is the same press again, not a repeat.
    assert_eq!(repeat(Some(&first), &at(1_000, (10, 10)), LIMITS), 1);
}

#[test]
fn releases_report_their_press_count() {
    let held = Modifiers::default();
    let first = press(0, (500, 500), &held);
    assert_eq!(release(0), first);
    // A second press at once and in place repeats the first.
    let second = press(0, (500, 500), &held);
    assert_eq!(second, first + 1);
    assert_eq!(release(0), second);
    assert_eq!(release(2), 1);
    // Another button starts a new count.
    assert_eq!(press(2, (500, 500), &held), 1);
}
