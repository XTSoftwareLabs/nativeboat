use super::*;
use alloc::rc::Rc;
use core::cell::Cell;

fn method(realm: &mut Realm, exports: &Object, name: &str, args: &[Value]) -> Value {
    let function = ops::get(realm, &Value::Object(exports.clone()), &Key::name(name)).unwrap();
    ops::call(realm, &function, Value::Undefined, args).unwrap()
}

#[test]
fn public_keyboard_layout_api_returns_native_windows_data() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let names = exports
        .own_keys()
        .into_iter()
        .filter_map(|key| match key {
            Key::String(name) => Some(name.to_rust_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "getKeyMap",
            "getCurrentKeyboardLayout",
            "onDidChangeKeyboardLayout",
            "isISOKeyboard"
        ]
    );

    let map = method(&mut realm, &exports, "getKeyMap", &[])
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(map.own_keys().len(), 146);
    for name in ["KeyA", "Digit1", "Enter", "NumpadEnter"] {
        let entry = ops::get(&mut realm, &Value::Object(map.clone()), &Key::name(name)).unwrap();
        let entry = entry.as_object().expect("known physical key");
        for field in ["vkey", "value", "withShift", "withAltGr", "withShiftAltGr"] {
            assert!(entry.own_property(&Key::name(field)).is_some());
        }
    }

    let layout = method(&mut realm, &exports, "getCurrentKeyboardLayout", &[]);
    let layout = layout.as_object().expect("Windows layout metadata");
    for field in ["id", "text", "name"] {
        assert!(layout.own_property(&Key::name(field)).is_some());
    }
    for field in ["text", "name"] {
        let value = ops::get(
            &mut realm,
            &Value::Object(layout.clone()),
            &Key::name(field),
        )
        .unwrap();
        assert!(matches!(value, Value::String(ref text) if !text.is_empty()));
    }
    assert!(matches!(
        method(&mut realm, &exports, "isISOKeyboard", &[Value::Undefined]),
        Value::Undefined
    ));
}

#[test]
fn registered_keyboard_change_callbacks_run_on_the_realm_thread() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let callback = realm.function("callback", 0, false, move |_, _, _| {
        observed.set(observed.get() + 1);
        Ok(Value::Undefined)
    });
    method(
        &mut realm,
        &exports,
        "onDidChangeKeyboardLayout",
        &[Value::Object(callback)],
    );
    assert_eq!(realm.keyboard_layout_callbacks.len(), 1);
    // SAFETY: the pointer refers to this live Realm for the duration of dispatch.
    unsafe { layout_changed((&mut realm as *mut Realm).cast()) };
    assert_eq!(calls.get(), 1);
}

#[test]
fn keyboard_change_registration_rejects_non_functions() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let function = ops::get(
        &mut realm,
        &Value::Object(exports),
        &Key::name("onDidChangeKeyboardLayout"),
    )
    .unwrap();
    for argument in [Value::str("callback"), Value::Object(realm.object())] {
        let thrown = ops::call(&mut realm, &function, Value::Undefined, &[argument]).unwrap_err();
        assert_eq!(
            realm.uncaught(&thrown.0),
            "Uncaught TypeError: Wrong type of arguments. Expects a function as first argument.\n"
        );
    }
}

#[test]
fn missing_keyboard_layout_registry_entry_returns_an_empty_string() {
    assert!(registry("missing-layout-for-test", "Layout Text").is_empty());
}

#[test]
fn keyboard_layout_dispatch_ignores_a_null_realm() {
    // SAFETY: layout_changed explicitly accepts null as a no-op sentinel.
    unsafe { layout_changed(core::ptr::null_mut()) };
}
