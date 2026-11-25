use super::*;

#[test]
fn reports_exit_reasons_only_when_enabled() {
    // SAFETY: no other test reads or writes this variable.
    unsafe { std::env::remove_var("NATIVEBOAT_TRACE_EXIT") };
    assert!(!enabled());
    exit("not reported");
    // SAFETY: as above.
    unsafe { std::env::set_var("NATIVEBOAT_TRACE_EXIT", "1") };
    assert!(enabled());
    exit("reported");
    // SAFETY: as above.
    unsafe { std::env::remove_var("NATIVEBOAT_TRACE_EXIT") };
}

#[test]
fn thrown_values_are_described_without_running_scripts() {
    let realm = crate::Realm::new();
    let error = realm.throw(crate::ErrorKind::Type, "x is not a function").0;
    assert_eq!(describe_thrown(&error), "TypeError: x is not a function");
    assert_eq!(
        describe_thrown(&crate::Value::str("plain")),
        "string \"plain\""
    );
    assert_eq!(describe_thrown(&crate::Value::Number(3.0)), "Number(3.0)");
    // A getter for the message is not run.
    let object = realm.object();
    object.define(
        crate::Key::name("message"),
        crate::Property {
            slot: crate::object::Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    assert_eq!(describe_thrown(&crate::Value::Object(object)), ": ");
    // Tracing is off unless asked for; the result is read once.
    // SAFETY: no other test reads or writes this variable.
    unsafe { std::env::remove_var("NATIVEBOAT_TRACE_THROWS") };
    assert!(!throws_traced());
    thrown(&error);
    assert!(matches!(
        crate::ops::thrown(crate::Value::Null).0,
        crate::Value::Null
    ));
}

#[test]
fn call_profiles_count_and_rank_call_sites_by_time() {
    // Reports wait five seconds after the first record.
    let mut profile = super::CallProfile {
        reported: crate::platform::now(),
        ..super::CallProfile::default()
    };
    let fast = "fast";
    let slow = "this.slow";
    // `slow` calls `fast` three times: its own time leaves them out.
    profile.enter();
    for _ in 0..3 {
        profile.enter();
        profile.record(fast, 1.0);
    }
    profile.record(slow, 10.0);
    let report = profile.report(5);
    let lines: Vec<&str> = report.lines().collect();
    assert_eq!(
        lines[0],
        "nativeboat: calls by site (own ms, total ms, count)"
    );
    assert!(
        lines[1].ends_with("7.0       10.0        1 this.slow"),
        "{report}"
    );
    assert!(
        lines[2].ends_with("3.0        3.0        3 fast"),
        "{report}"
    );
    assert_eq!(profile.report(1).lines().count(), 2);
}
