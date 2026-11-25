use super::*;

#[test]
fn the_strongest_started_blocker_decides_the_execution_state() {
    assert_eq!(execution_state(&[]), ES_CONTINUOUS);
    assert_eq!(
        execution_state(&[(0, false)]),
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    );
    assert_eq!(
        execution_state(&[(0, false), (1, true)]),
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
    );
}

#[test]
fn blockers_start_stop_and_report_by_number() {
    let mut realm = Realm::new();
    let blocker = module(&realm);
    let call = |realm: &mut Realm, name: &str, args: &[Value]| {
        let method = ops::get(realm, &Value::Object(blocker.clone()), &Key::name(name)).unwrap();
        ops::call(realm, &method, Value::Object(blocker.clone()), args)
    };
    let first = call(&mut realm, "start", &[Value::str("prevent-app-suspension")]).unwrap();
    let second = call(&mut realm, "start", &[Value::str("prevent-display-sleep")]).unwrap();
    assert!(matches!(first, Value::Number(0.0)));
    assert!(matches!(second, Value::Number(1.0)));
    assert!(matches!(
        call(&mut realm, "isStarted", std::slice::from_ref(&first)).unwrap(),
        Value::Boolean(true)
    ));
    assert!(matches!(
        call(&mut realm, "stop", std::slice::from_ref(&first)).unwrap(),
        Value::Boolean(true)
    ));
    assert!(matches!(
        call(&mut realm, "stop", std::slice::from_ref(&first)).unwrap(),
        Value::Boolean(false)
    ));
    assert!(matches!(
        call(&mut realm, "isStarted", &[first]).unwrap(),
        Value::Boolean(false)
    ));
    assert!(call(&mut realm, "start", &[Value::str("stay-up")]).is_err());
    assert!(call(&mut realm, "start", &[]).is_err());
    // Numbers are not given out again.
    assert!(matches!(
        call(&mut realm, "start", &[Value::str("prevent-app-suspension")]).unwrap(),
        Value::Number(2.0)
    ));
    for id in [1.0, 2.0] {
        call(&mut realm, "stop", &[Value::Number(id)]).unwrap();
    }
}
