use super::*;

fn time(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16) -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: year,
        wMonth: month,
        wDay: day,
        wHour: hour,
        wMinute: minute,
        wSecond: second,
        ..SYSTEMTIME::default()
    }
}

#[test]
fn pictures_follow_the_locale() {
    set_date_locale("en-US");
    set_date_locale("en-US");
    assert_eq!(picture(ControlKind::Date, false), "MM/dd/yyyy");
    assert_eq!(picture(ControlKind::Date, true), "'mm/dd/yyyy'");
    assert_eq!(picture(ControlKind::Time, false), "hh:mm tt");
    assert_eq!(picture(ControlKind::Time, true), "'--:-- --'");
    assert_eq!(
        picture(ControlKind::DateTime, false),
        "MM/dd/yyyy', 'hh:mm tt"
    );
    assert_eq!(
        picture(ControlKind::DateTime, true),
        "'mm/dd/yyyy', '--:-- --'"
    );
    assert_eq!(picture(ControlKind::Month, false), "MMMM yyyy");
    assert_eq!(picture(ControlKind::Month, true), "'---- ----'");
    set_date_locale("de-DE");
    assert_eq!(picture(ControlKind::Date, false), "dd.MM.yyyy");
    assert_eq!(picture(ControlKind::Time, false), "HH:mm");
    // Without data the American pictures apply.
    set_date_locale("not a locale");
    assert_eq!(picture(ControlKind::Date, false), "MM/dd/yyyy");
    set_date_locale("en-US");
    assert_eq!(padded("d'.'M'.'yy H:m t"), "dd'.'MM'.'yyyy HH:mm tt");
    assert_eq!(padded("'d' d"), "'d' dd");
    assert_eq!(placeholder("d 'de' MMMM", false), "'d de mmmm'");
    assert_eq!(placeholder("h'h'mm", true), "'-h--'");
}

#[test]
fn values_convert_to_times_and_back() {
    let date = system_time(ControlKind::Date, "2024-03-15").unwrap();
    assert_eq!((date.wYear, date.wMonth, date.wDay), (2024, 3, 15));
    assert_eq!(value(ControlKind::Date, &date), "2024-03-15");
    assert_eq!(
        system_time(ControlKind::Date, "2024-02-29").unwrap().wDay,
        29
    );
    assert_eq!(
        system_time(ControlKind::Date, "2000-02-29").unwrap().wDay,
        29
    );
    for invalid in [
        "",
        "2023-02-29",
        "1900-02-29",
        "2024-04-31",
        "2024-13-01",
        "2024-00-10",
        "2024-01-00",
        "1600-01-01",
        "2024-01",
        "x-1-1",
        "2024-1-1-1",
    ] {
        assert!(
            system_time(ControlKind::Date, invalid).is_none(),
            "{invalid}"
        );
    }
    let clock = system_time(ControlKind::Time, "09:05").unwrap();
    assert_eq!((clock.wHour, clock.wMinute, clock.wSecond), (9, 5, 0));
    assert_eq!(value(ControlKind::Time, &clock), "09:05");
    assert_eq!(
        system_time(ControlKind::Time, "23:59:30.250")
            .unwrap()
            .wSecond,
        30
    );
    for invalid in ["9", "24:00", "12:60", "12:00:60", "1:2:3:4", "a:b"] {
        assert!(
            system_time(ControlKind::Time, invalid).is_none(),
            "{invalid}"
        );
    }
    let moment = system_time(ControlKind::DateTime, "2024-03-15T13:45").unwrap();
    assert_eq!(value(ControlKind::DateTime, &moment), "2024-03-15T13:45");
    assert_eq!(
        system_time(ControlKind::DateTime, "2024-03-15 13:45")
            .unwrap()
            .wHour,
        13
    );
    assert!(system_time(ControlKind::DateTime, "2024-03-15").is_none());
    assert!(system_time(ControlKind::DateTime, "2024-03-15T").is_none());
    let month = system_time(ControlKind::Month, "2024-03").unwrap();
    assert_eq!(value(ControlKind::Month, &month), "2024-03");
    assert!(system_time(ControlKind::Month, "2024-03-01").is_none());
    assert_eq!(
        value(ControlKind::Date, &time(1999, 12, 31, 23, 59, 0)),
        "1999-12-31"
    );
}
