use super::*;

#[test]
fn windows_describes_locales() {
    let german = locale_data("de-DE").unwrap();
    assert_eq!(german["name"], "de-DE");
    assert_eq!(german["months"][4], "Mai");
    assert_eq!(german["weekdays"][0], "Sonntag");
    assert_eq!(german["decimal"], ",");
    assert_eq!(german["group"], ".");
    assert_eq!(german["currency"], "EUR");
    // Months as dates inflect them.
    let russian = locale_data("ru-RU").unwrap();
    assert_eq!(russian["monthsGenitive"][4], "мая");
    assert_eq!(russian["months"][4], "Май");
    assert!(locale_data("xx-ZZ-invalid1").is_none());
    assert_eq!(text(&[], 0), "");
}

#[test]
fn time_zones_follow_windows_rules() {
    assert!(!local_time_zone().is_empty());
    assert_eq!(zone_offset("UTC", 0.0), Some(0.0));
    // Pacific time is eight hours behind UTC in January 2026, seven in July.
    assert_eq!(
        zone_offset("Pacific Standard Time", 1_767_225_600_000.0),
        Some(-480.0)
    );
    assert_eq!(
        zone_offset("pacific standard time", 1_782_864_000_000.0),
        Some(-420.0)
    );
    assert_eq!(
        zone_offset("India Standard Time", 1_767_225_600_000.0),
        Some(330.0)
    );
    assert_eq!(zone_offset("No Such Zone", 0.0), None);
    assert_eq!(zone_offset("UTC", -1.0e17), None);
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    assert_eq!(days_from_civil(1969, 12, 31), -1);
}
