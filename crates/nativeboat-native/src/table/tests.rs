use super::*;
use alloc::format;
use alloc::string::String;

#[test]
fn maps_insert_replace_find_and_remove_through_collisions_and_growth() {
    let mut map = HashMap::with_seed(0x1234_5678);
    assert_eq!(map.get(&1u64), None);
    assert_eq!(map.remove(&1u64), None);
    for key in 0..2_000u64 {
        assert_eq!(map.insert(key, key * 2), None);
    }
    assert_eq!(map.insert(7, 70), Some(14));
    assert_eq!(map.get(&7), Some(&70));
    assert_eq!(map.get(&5_000), None);
    assert_eq!(map.remove(&5_000), None);
    // Removing every other key shifts later probe-run entries back; the rest
    // stay reachable.
    for key in (0..2_000u64).step_by(2) {
        assert!(map.remove(&key).is_some());
    }
    for key in 0..2_000u64 {
        let expected = if key % 2 == 0 {
            None
        } else if key == 7 {
            Some(70)
        } else {
            Some(key * 2)
        };
        assert_eq!(map.get(&key).copied(), expected, "{key}");
    }
    for key in (1..2_000u64).step_by(2) {
        assert!(map.remove(&key).is_some());
    }
    assert_eq!(map.get(&1), None);
    assert_eq!(map.remove(&1), None);
}

#[test]
fn maps_hash_string_keys_and_format_their_entries() {
    let mut map: HashMap<String, usize> = HashMap::new();
    for index in 0..100 {
        map.insert(format!("key-{index}-with-a-longer-name"), index);
    }
    assert_eq!(
        map.get(&String::from("key-42-with-a-longer-name")),
        Some(&42)
    );
    let mut small = HashMap::with_seed(1);
    small.insert("a", 1);
    assert_eq!(format!("{small:?}"), "{\"a\": 1}");
}

#[test]
fn sets_report_whether_keys_were_new() {
    let mut set = HashSet::new();
    assert!(set.insert(3));
    assert!(!set.insert(3));
    assert!(set.insert(4));
}
