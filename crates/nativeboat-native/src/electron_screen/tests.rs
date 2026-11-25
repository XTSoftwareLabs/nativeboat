use super::*;

fn property(realm: &mut Realm, object: &Object, name: &str) -> Value {
    ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
}

#[test]
fn screen_reports_native_displays_and_cursor_position() {
    let mut realm = Realm::new();
    let screen = module(&realm);
    let primary = property(&mut realm, &screen, "getPrimaryDisplay");
    let primary = ops::call(&mut realm, &primary, Value::Object(screen.clone()), &[]).unwrap();
    let bounds = property(&mut realm, primary.as_object().unwrap(), "bounds");
    let width = property(&mut realm, bounds.as_object().unwrap(), "width");
    let height = property(&mut realm, bounds.as_object().unwrap(), "height");
    assert!(matches!(width, Value::Number(width) if width > 0.0));
    assert!(matches!(height, Value::Number(height) if height > 0.0));
    assert!(matches!(
        property(&mut realm, primary.as_object().unwrap(), "scaleFactor"),
        Value::Number(scale) if scale >= 0.5
    ));

    let all = property(&mut realm, &screen, "getAllDisplays");
    let all = ops::call(&mut realm, &all, Value::Object(screen.clone()), &[]).unwrap();
    let length = ops::get(&mut realm, &all, &Key::name("length")).unwrap();
    assert!(matches!(length, Value::Number(length) if length >= 1.0));

    let cursor = property(&mut realm, &screen, "getCursorScreenPoint");
    let cursor = ops::call(&mut realm, &cursor, Value::Object(screen.clone()), &[]).unwrap();
    assert!(matches!(
        property(&mut realm, cursor.as_object().unwrap(), "x"),
        Value::Number(_)
    ));
    assert!(matches!(
        property(&mut realm, cursor.as_object().unwrap(), "y"),
        Value::Number(_)
    ));
}

#[test]
fn screen_selects_the_nearest_and_matching_display() {
    let mut realm = Realm::new();
    let screen = module(&realm);
    let primary_method = property(&mut realm, &screen, "getPrimaryDisplay");
    let primary = ops::call(
        &mut realm,
        &primary_method,
        Value::Object(screen.clone()),
        &[],
    )
    .unwrap();
    let bounds = property(&mut realm, primary.as_object().unwrap(), "bounds");
    let x = property(&mut realm, bounds.as_object().unwrap(), "x");
    let y = property(&mut realm, bounds.as_object().unwrap(), "y");
    let width = property(&mut realm, bounds.as_object().unwrap(), "width");
    let height = property(&mut realm, bounds.as_object().unwrap(), "height");
    let point = realm.object();
    point.define(Key::name("x"), Property::data(x.clone()));
    point.define(Key::name("y"), Property::data(y.clone()));

    let nearest_method = property(&mut realm, &screen, "getDisplayNearestPoint");
    let nearest = ops::call(
        &mut realm,
        &nearest_method,
        Value::Object(screen.clone()),
        &[Value::Object(point.clone())],
    )
    .unwrap();
    assert!(matches!(
        property(&mut realm, nearest.as_object().unwrap(), "id"),
        Value::Number(_)
    ));

    let matching_method = property(&mut realm, &screen, "getDisplayMatching");
    let matching_rect = realm.object();
    for (name, value) in [("x", x), ("y", y), ("width", width), ("height", height)] {
        matching_rect.define(Key::name(name), Property::data(value));
    }
    let matching = ops::call(
        &mut realm,
        &matching_method,
        Value::Object(screen.clone()),
        &[Value::Object(matching_rect)],
    )
    .unwrap();
    let primary_id = property(&mut realm, primary.as_object().unwrap(), "id");
    let matching_id = property(&mut realm, matching.as_object().unwrap(), "id");
    assert!(crate::same_value(&primary_id, &matching_id));
    assert!(
        ops::call(
            &mut realm,
            &nearest_method,
            Value::Object(screen),
            &[Value::Undefined],
        )
        .is_err()
    );
}

#[test]
fn geometry_handles_scaled_negative_bounds_and_all_edges() {
    let scaled = logical_rect(
        Rect {
            left: -3840,
            top: -1080,
            right: 0,
            bottom: 0,
        },
        2.0,
    );
    assert_eq!(scaled, (-1920, -540, 1920, 540));
    assert_eq!(logical_rect(Rect::default(), 0.25), (0, 0, 0, 0));

    let displays = [
        Display {
            id: 1,
            bounds: (-10, -10, 10, 10),
            work_area: (-10, -10, 10, 9),
            scale: 1.0,
            primary: true,
        },
        Display {
            id: 2,
            bounds: (10, 0, 20, 20),
            work_area: (10, 0, 20, 19),
            scale: 1.5,
            primary: false,
        },
    ];

    assert_eq!(nearest(&displays, 25.0, 5.0).id, 2);
    assert_eq!(nearest(&displays, -20.0, -20.0).id, 1);
    assert_eq!(nearest(&displays, 5.0, 30.0).id, 2);
    assert_eq!(matching(&displays, (8.0, -2.0, 16.0, 12.0)).id, 2);
    assert_eq!(matching(&displays, (40.0, 40.0, 2.0, 2.0)).id, 2);
}

#[test]
fn display_fallback_is_created_only_when_monitor_enumeration_is_empty() {
    let mut displays = Vec::new();
    ensure_display_fallback(&mut displays);
    assert_eq!(displays.len(), 1);
    assert_eq!(displays[0].id, 0);
    assert!(displays[0].primary);
    assert_eq!(displays[0].scale, 1.0);
    assert!(displays[0].bounds.2 > 0);
    assert!(displays[0].bounds.3 > 0);

    let primary = displays[0];
    ensure_display_fallback(&mut displays);
    assert_eq!(displays.len(), 1);
    assert_eq!(displays[0].bounds, primary.bounds);
}
