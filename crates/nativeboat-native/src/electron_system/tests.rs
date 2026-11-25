use super::*;
use crate::app_host::NotifyEvent;
use alloc::rc::Rc;
use core::cell::RefCell;

fn call(realm: &mut Realm, module: &Object, name: &str, args: &[Value]) -> Result<Value, Thrown> {
    let method = ops::get(realm, &Value::Object(module.clone()), &Key::name(name)).unwrap();
    ops::call(realm, &method, Value::Object(module.clone()), args)
}

fn text(value: Value) -> String {
    match value {
        Value::String(text) => text.to_rust_lossy(),
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
fn colors_are_written_as_electron_writes_them() {
    // DWM's 0xAARRGGBB becomes rrggbbaa; a COLORREF's 0x00BBGGRR #RRGGBBFF.
    assert_eq!(accent_hex(0xc400_78d7), "0078d7c4");
    assert_eq!(system_color_hex(0x00d7_7800), "#0078D7FF");
    let index = |name: &str| {
        SYSTEM_COLORS
            .iter()
            .find(|(known, _)| *known == name)
            .unwrap()
            .1
    };
    assert_eq!(
        (
            index("window"),
            index("highlight"),
            index("menubar"),
            index("scrollbar")
        ),
        (5, 13, 30, 0)
    );
}

#[test]
fn power_monitor_reports_idle_time_state_and_power() {
    let mut realm = Realm::new();
    let monitor = power_monitor(&mut realm, None);
    assert!(
        call(
            &mut realm,
            &monitor,
            "getSystemIdleState",
            &[Value::Number(0.0)]
        )
        .is_err()
    );
    assert!(call(&mut realm, &monitor, "getSystemIdleState", &[]).is_err());
    let state = text(
        call(
            &mut realm,
            &monitor,
            "getSystemIdleState",
            &[Value::Number(60.0)],
        )
        .unwrap(),
    );
    assert!(
        ["active", "idle", "locked"].contains(&state.as_str()),
        "{state}"
    );
    assert!(matches!(
        call(&mut realm, &monitor, "getSystemIdleTime", &[]).unwrap(),
        Value::Number(seconds) if seconds >= 0.0
    ));
    assert_eq!(
        text(call(&mut realm, &monitor, "getCurrentThermalState", &[]).unwrap()),
        "unknown"
    );
    let battery = call(&mut realm, &monitor, "isOnBatteryPower", &[]).unwrap();
    let property = ops::get(
        &mut realm,
        &Value::Object(monitor),
        &Key::name("onBatteryPower"),
    )
    .unwrap();
    assert!(matches!((battery, property), (Value::Boolean(a), Value::Boolean(b)) if a == b));
    assert!(realm.notify_targets.contains_key(&POWER_MONITOR));
}

#[test]
fn system_preferences_answer_as_on_windows() {
    let mut realm = Realm::new();
    let preferences = system_preferences(&mut realm, None);
    let window = text(
        call(
            &mut realm,
            &preferences,
            "getColor",
            &[Value::str("window")],
        )
        .unwrap(),
    );
    assert!(
        window.starts_with('#') && window.ends_with("FF") && window.len() == 9,
        "{window}"
    );
    assert!(
        call(
            &mut realm,
            &preferences,
            "getColor",
            &[Value::str("no-such-color")]
        )
        .is_err()
    );
    let accent = text(call(&mut realm, &preferences, "getAccentColor", &[]).unwrap());
    assert!(accent.is_empty() || accent.len() == 8, "{accent}");
    assert_eq!(
        text(
            call(
                &mut realm,
                &preferences,
                "getMediaAccessStatus",
                &[Value::str("screen")]
            )
            .unwrap()
        ),
        "granted"
    );
    let camera = text(
        call(
            &mut realm,
            &preferences,
            "getMediaAccessStatus",
            &[Value::str("camera")],
        )
        .unwrap(),
    );
    assert!(["granted", "denied"].contains(&camera.as_str()));
    assert!(
        call(
            &mut realm,
            &preferences,
            "getMediaAccessStatus",
            &[Value::str("speaker")]
        )
        .is_err()
    );
    // Electron 35 removed isAeroGlassEnabled; earlier releases have it.
    assert!(matches!(
        ops::get(
            &mut realm,
            &Value::Object(preferences.clone()),
            &Key::name("isAeroGlassEnabled")
        )
        .unwrap(),
        Value::Undefined
    ));
    let mut older = Realm::new();
    older.electron_version = Some("30.5.1");
    let earlier = system_preferences(&mut older, None);
    assert!(matches!(
        call(&mut older, &earlier, "isAeroGlassEnabled", &[]).unwrap(),
        Value::Boolean(true)
    ));
    // Each release reads the accent color its own way, both as rrggbbaa.
    for major in [30, 38] {
        let color = accent_color(major).map(accent_hex).unwrap_or_default();
        assert!(color.is_empty() || color.len() == 8, "{major} {color}");
    }
    let Value::Object(settings) =
        call(&mut realm, &preferences, "getAnimationSettings", &[]).unwrap()
    else {
        panic!("settings must be an object")
    };
    let get = |realm: &mut Realm, name: &str| {
        ops::get(realm, &Value::Object(settings.clone()), &Key::name(name)).unwrap()
    };
    let (rich, reduced) = (
        get(&mut realm, "shouldRenderRichAnimation"),
        get(&mut realm, "prefersReducedMotion"),
    );
    assert!(matches!((rich, reduced), (Value::Boolean(a), Value::Boolean(b)) if a != b));
}

#[test]
fn system_events_reach_the_modules_with_an_event_first() {
    let mut realm = Realm::new();
    let heard: Rc<RefCell<Vec<String>>> = Rc::default();
    for target in [POWER_MONITOR, SYSTEM_PREFERENCES] {
        let object = realm.object();
        let log = heard.clone();
        let emit = realm.function("emit", 1, false, move |realm, _, args| {
            let name = ops::to_string(realm, &args[0])?.to_rust_lossy();
            let event = matches!(args.get(1), Some(Value::Object(_)));
            let detail = match args.get(2) {
                Some(value) => alloc::format!(" {}", ops::to_string(realm, value)?.to_rust_lossy()),
                None => String::new(),
            };
            log.borrow_mut()
                .push(alloc::format!("{name} {event}{detail}"));
            Ok(Value::Boolean(true))
        });
        object.define(Key::name("emit"), Property::data(Value::Object(emit)));
        realm.notify_targets.insert(
            target,
            crate::electron_notify::Target::System {
                object,
                accent: Some(0xff00_0000),
            },
        );
    }
    let context = (&raw mut realm).cast();
    for (event, id) in [
        (NotifyEvent::Suspend, 0),
        (NotifyEvent::Resume, 0),
        (NotifyEvent::OnBattery, 0),
        (NotifyEvent::OnAc, 0),
        (NotifyEvent::LockScreen, 0),
        (NotifyEvent::UnlockScreen, 0),
        (NotifyEvent::AccentColor, 0xc400_78d7),
        // The same color again is no change.
        (NotifyEvent::AccentColor, 0xc400_78d7),
        (NotifyEvent::SystemColors, 0),
    ] {
        // SAFETY: the context is this test's live realm.
        unsafe { crate::electron_notify::notify_event(context, event, id) };
    }
    assert_eq!(
        heard.borrow().as_slice(),
        [
            "suspend true",
            "resume true",
            "on-battery true",
            "on-ac true",
            "lock-screen true",
            "unlock-screen true",
            "accent-color-changed true 0078d7c4",
            "color-changed true",
        ]
    );
}
