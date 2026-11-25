use super::*;

fn performance(realm: &mut Realm) -> Value {
    let global = Value::Object(realm.global_object());
    ops::get_named(realm, &global, "performance").unwrap()
}

fn call(realm: &mut Realm, name: &str, args: &[Value]) -> Result<Value, Thrown> {
    let performance = performance(realm);
    let method = ops::get(realm, &performance, &Key::name(name)).unwrap();
    ops::call(realm, &method, performance, args)
}

fn number(realm: &mut Realm, value: &Value, name: &str) -> f64 {
    match ops::get(realm, value, &Key::name(name)).unwrap() {
        Value::Number(number) => number,
        other => panic!("{name}: {other:?}"),
    }
}

fn names(realm: &mut Realm, list: &Value) -> Vec<String> {
    let count = crate::array_builtin::length_of(realm, list).unwrap();
    (0..count)
        .map(|index| {
            let entry = ops::get(realm, list, &Key::Index(index as u32)).unwrap();
            format!(
                "{} {}",
                string_of(realm, &entry, "entryType"),
                string_of(realm, &entry, "name")
            )
        })
        .collect()
}

#[test]
fn time_counts_from_the_realm_start_and_the_wall_clock_origin() {
    let mut realm = Realm::new();
    let first = match call(&mut realm, "now", &[]).unwrap() {
        Value::Number(now) => now,
        other => panic!("{other:?}"),
    };
    let second = match call(&mut realm, "now", &[]).unwrap() {
        Value::Number(now) => now,
        other => panic!("{other:?}"),
    };
    assert!(first >= 0.0 && second >= first, "{first} {second}");
    let performance = performance(&mut realm);
    let origin = number(&mut realm, &performance, "timeOrigin");
    assert!(
        (origin - crate::date_builtin::now()).abs() < 60_000.0,
        "{origin}"
    );
    let json = call(&mut realm, "toJSON", &[]).unwrap();
    assert_eq!(number(&mut realm, &json, "timeOrigin"), origin);
}

#[test]
fn marks_and_measures_are_kept_found_and_cleared() {
    let mut realm = Realm::new();
    let options = realm.object();
    options.define(Key::name("startTime"), Property::data(Value::Number(5.0)));
    options.define(Key::name("detail"), Property::data(Value::str("d")));
    let begin = call(
        &mut realm,
        "mark",
        &[Value::str("begin"), Value::Object(options)],
    )
    .unwrap();
    assert_eq!(number(&mut realm, &begin, "startTime"), 5.0);
    assert_eq!(string_of(&mut realm, &begin, "detail"), "d");
    let end = call(&mut realm, "mark", &[Value::str("end")]).unwrap();
    let end_time = number(&mut realm, &end, "startTime");
    assert!(matches!(
        ops::get_named(&mut realm, &end, "detail").unwrap(),
        Value::Null
    ));
    // Between marks, from a time, or from the start to now.
    let between = call(
        &mut realm,
        "measure",
        &[Value::str("span"), Value::str("begin"), Value::str("end")],
    )
    .unwrap();
    assert_eq!(number(&mut realm, &between, "duration"), end_time - 5.0);
    let from = call(
        &mut realm,
        "measure",
        &[Value::str("from"), Value::Number(2.0)],
    )
    .unwrap();
    assert_eq!(number(&mut realm, &from, "startTime"), 2.0);
    let whole = call(&mut realm, "measure", &[Value::str("whole")]).unwrap();
    assert_eq!(number(&mut realm, &whole, "startTime"), 0.0);
    // An options object with a duration, or an end mark.
    let timed = realm.object();
    timed.define(Key::name("start"), Property::data(Value::str("begin")));
    timed.define(Key::name("duration"), Property::data(Value::Number(3.0)));
    timed.define(Key::name("detail"), Property::data(Value::Boolean(true)));
    let measured = call(
        &mut realm,
        "measure",
        &[Value::str("timed"), Value::Object(timed)],
    )
    .unwrap();
    assert_eq!(number(&mut realm, &measured, "duration"), 3.0);
    let ended = realm.object();
    ended.define(Key::name("end"), Property::data(Value::str("end")));
    let measured = call(
        &mut realm,
        "measure",
        &[Value::str("ended"), Value::Object(ended)],
    )
    .unwrap();
    assert_eq!(number(&mut realm, &measured, "duration"), end_time);
    // Unknown marks and missing names throw.
    assert!(
        call(
            &mut realm,
            "measure",
            &[Value::str("x"), Value::str("nowhere")]
        )
        .is_err()
    );
    assert!(call(&mut realm, "mark", &[]).is_err());
    assert!(call(&mut realm, "measure", &[Value::Undefined]).is_err());
    let all = call(&mut realm, "getEntries", &[]).unwrap();
    assert_eq!(names(&mut realm, &all).len(), 7);
    let marks = call(&mut realm, "getEntriesByType", &[Value::str("mark")]).unwrap();
    assert_eq!(names(&mut realm, &marks), ["mark begin", "mark end"]);
    let spans = call(&mut realm, "getEntriesByName", &[Value::str("span")]).unwrap();
    assert_eq!(names(&mut realm, &spans), ["measure span"]);
    let typed = call(
        &mut realm,
        "getEntriesByName",
        &[Value::str("span"), Value::str("mark")],
    )
    .unwrap();
    assert!(names(&mut realm, &typed).is_empty());
    call(&mut realm, "clearMarks", &[Value::str("begin")]).unwrap();
    let marks = call(&mut realm, "getEntriesByType", &[Value::str("mark")]).unwrap();
    assert_eq!(names(&mut realm, &marks), ["mark end"]);
    call(&mut realm, "clearMeasures", &[]).unwrap();
    call(&mut realm, "clearMarks", &[Value::Undefined]).unwrap();
    let all = call(&mut realm, "getEntries", &[]).unwrap();
    assert!(names(&mut realm, &all).is_empty());
}
