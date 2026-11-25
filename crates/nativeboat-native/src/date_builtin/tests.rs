//! Expected values are Node 22's for the same operations, with the local
//! time zone and locale injected: a fixed UTC+8 zone (this machine's), and
//! US Pacific time, whose 2026 transitions are March 8 and November 1.

use super::*;

fn s(text: &str) -> Value {
    Value::str(text)
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn name(text: &str) -> [u16; 32] {
    let mut units = [0u16; 32];
    for (slot, unit) in units.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
    }
    units
}

fn shanghai() -> TimeZone {
    TimeZone {
        bias: -480,
        standard_name: name("China Standard Time"),
        daylight_name: name("China Daylight Time"),
        dynamic_daylight_time_disabled: 1,
        ..TimeZone::default()
    }
}

fn pacific() -> TimeZone {
    TimeZone {
        bias: 480,
        standard_name: name("Pacific Standard Time"),
        standard_date: SystemTime {
            month: 11,
            day: 1,
            hour: 2,
            ..SystemTime::default()
        },
        daylight_name: name("Pacific Daylight Time"),
        daylight_date: SystemTime {
            month: 3,
            day: 2,
            hour: 2,
            ..SystemTime::default()
        },
        daylight_bias: -60,
        dynamic_daylight_time_disabled: 0,
        ..TimeZone::default()
    }
}

fn realm_with(zone: TimeZone, locale: &str) -> Realm {
    let mut realm = Realm::new();
    realm.time_zone = Some(zone);
    realm.locale = Some(locale.encode_utf16().chain([0]).collect());
    realm
}

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

fn date(realm: &mut Realm, args: &[Value]) -> Value {
    let constructor = global(realm, "Date");
    ops::construct(realm, &constructor, args).unwrap()
}

fn statics(realm: &mut Realm, name: &str, args: &[Value]) -> f64 {
    let constructor = global(realm, "Date");
    let function = ops::get(realm, &constructor, &Key::name(name)).unwrap();
    let result = ops::call(realm, &function, constructor, args).unwrap();
    ops::to_number(realm, &result).unwrap()
}

fn utc(realm: &mut Realm, parts: &[f64]) -> f64 {
    let args: Vec<Value> = parts.iter().copied().map(n).collect();
    statics(realm, "UTC", &args)
}

fn call(realm: &mut Realm, target: &Value, method: &str, args: &[Value]) -> String {
    let function = ops::get(realm, target, &Key::name(method)).unwrap();
    match ops::call(realm, &function, target.clone(), args) {
        Ok(value) => ops::to_string(realm, &value).unwrap().to_rust_lossy(),
        Err(thrown) => format!(
            "throws {}",
            ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
        ),
    }
}

fn parse(realm: &mut Realm, text: &str) -> f64 {
    statics(realm, "parse", &[s(text)])
}

#[test]
fn formats_follow_v8_in_a_fixed_zone() {
    let mut realm = realm_with(shanghai(), "zh-CN");
    let time = utc(&mut realm, &[2026.0, 9.0, 11.0, 7.0, 30.0, 5.0, 123.0]);
    assert_eq!(time, 1_791_703_805_123.0);
    let d = date(&mut realm, &[n(time)]);
    let expected = [
        (
            "toString",
            "Sun Oct 11 2026 15:30:05 GMT+0800 (China Standard Time)",
        ),
        ("toUTCString", "Sun, 11 Oct 2026 07:30:05 GMT"),
        ("toGMTString", "Sun, 11 Oct 2026 07:30:05 GMT"),
        ("toISOString", "2026-10-11T07:30:05.123Z"),
        ("toJSON", "2026-10-11T07:30:05.123Z"),
        ("toDateString", "Sun Oct 11 2026"),
        ("toTimeString", "15:30:05 GMT+0800 (China Standard Time)"),
        ("toLocaleString", "2026/10/11 15:30:05"),
        ("toLocaleDateString", "2026/10/11"),
        ("toLocaleTimeString", "15:30:05"),
        ("getTimezoneOffset", "-480"),
        ("valueOf", "1791703805123"),
        ("getTime", "1791703805123"),
        ("getFullYear", "2026"),
        ("getMonth", "9"),
        ("getDate", "11"),
        ("getDay", "0"),
        ("getHours", "15"),
        ("getMinutes", "30"),
        ("getSeconds", "5"),
        ("getMilliseconds", "123"),
        ("getUTCHours", "7"),
        ("getUTCDay", "0"),
        ("getYear", "126"),
    ];
    for (method, text) in expected {
        assert_eq!(call(&mut realm, &d, method, &[]), text, "{method}");
    }
    let rolled = date(&mut realm, &[n(2026.0), n(0.0), n(31.0), n(25.0)]);
    assert_eq!(
        call(&mut realm, &rolled, "toString", &[]),
        "Sun Feb 01 2026 01:00:00 GMT+0800 (China Standard Time)"
    );
    let invalid = date(&mut realm, &[n(f64::NAN)]);
    for method in ["toString", "toUTCString", "toDateString", "toLocaleString"] {
        assert_eq!(call(&mut realm, &invalid, method, &[]), "Invalid Date");
    }
    assert_eq!(
        call(&mut realm, &invalid, "toISOString", &[]),
        "throws RangeError: Invalid time value"
    );
    for method in [
        "getTime",
        "getHours",
        "getUTCHours",
        "getTimezoneOffset",
        "getYear",
    ] {
        assert_eq!(call(&mut realm, &invalid, method, &[]), "NaN", "{method}");
    }
    assert_eq!(call(&mut realm, &invalid, "toJSON", &[]), "null");
    // Years beyond four digits, before year zero and the time value limit.
    let far = utc(&mut realm, &[275_000.0, 0.0, 1.0]);
    let far = date(&mut realm, &[n(far)]);
    assert_eq!(
        call(&mut realm, &far, "toISOString", &[]),
        "+275000-01-01T00:00:00.000Z"
    );
    let ancient = date(&mut realm, &[n(-62_198_755_200_000.0)]);
    assert_eq!(
        call(&mut realm, &ancient, "toISOString", &[]),
        "-000001-01-01T00:00:00.000Z"
    );
    assert_eq!(
        call(&mut realm, &ancient, "toUTCString", &[]),
        "Fri, 01 Jan -0001 00:00:00 GMT"
    );
    assert_eq!(
        call(&mut realm, &ancient, "toString", &[]),
        "Fri Jan 01 -0001 08:00:00 GMT+0800 (China Standard Time)"
    );
    // Outside Windows' years, locale strings fall back to V8's format.
    assert_eq!(
        call(&mut realm, &ancient, "toLocaleString", &[]),
        "Fri Jan 01 -0001 08:00:00"
    );
    assert!(statics(&mut realm, "UTC", &[n(8.64e15 + 1.0)]).is_nan());
}

#[test]
fn local_time_follows_daylight_saving_transitions() {
    let mut realm = realm_with(pacific(), "en-US");
    let summer = utc(&mut realm, &[2026.0, 6.0, 1.0, 19.0, 5.0, 9.0, 7.0]);
    let winter = utc(&mut realm, &[2026.0, 0.0, 15.0, 12.0]);
    let summer = date(&mut realm, &[n(summer)]);
    let winter = date(&mut realm, &[n(winter)]);
    assert_eq!(
        call(&mut realm, &summer, "toString", &[]),
        "Wed Jul 01 2026 12:05:09 GMT-0700 (Pacific Daylight Time)"
    );
    assert_eq!(
        call(&mut realm, &winter, "toString", &[]),
        "Thu Jan 15 2026 04:00:00 GMT-0800 (Pacific Standard Time)"
    );
    assert_eq!(call(&mut realm, &summer, "getTimezoneOffset", &[]), "420");
    assert_eq!(call(&mut realm, &winter, "getTimezoneOffset", &[]), "480");
    assert_eq!(
        call(&mut realm, &summer, "toLocaleString", &[]),
        "7/1/2026, 12:05:09 PM"
    );
    assert_eq!(
        call(&mut realm, &summer, "toLocaleDateString", &[]),
        "7/1/2026"
    );
    assert_eq!(
        call(&mut realm, &summer, "toLocaleTimeString", &[]),
        "12:05:09 PM"
    );
    // A skipped local time uses the offset before the transition; a
    // repeated one is the earlier instant.
    let gap = date(&mut realm, &[n(2026.0), n(2.0), n(8.0), n(2.0), n(30.0)]);
    let gap_utc = utc(&mut realm, &[2026.0, 2.0, 8.0, 10.0, 30.0]);
    assert_eq!(call(&mut realm, &gap, "getTime", &[]), gap_utc.to_string());
    assert_eq!(
        call(&mut realm, &gap, "toString", &[]),
        "Sun Mar 08 2026 03:30:00 GMT-0700 (Pacific Daylight Time)"
    );
    let overlap = date(&mut realm, &[n(2026.0), n(10.0), n(1.0), n(1.0), n(30.0)]);
    let overlap_utc = utc(&mut realm, &[2026.0, 10.0, 1.0, 8.0, 30.0]);
    assert_eq!(
        call(&mut realm, &overlap, "getTime", &[]),
        overlap_utc.to_string()
    );
    // Years outside Windows' range use the rules 400·n years away.
    for year in [1000.0, 40_000.0] {
        let shifted = date(&mut realm, &[n(year), n(6.0), n(1.0)]);
        assert_eq!(
            call(&mut realm, &shifted, "getTimezoneOffset", &[]),
            "420",
            "{year}"
        );
    }
}

#[test]
fn parsing_accepts_iso_and_legacy_forms() {
    let mut realm = realm_with(shanghai(), "zh-CN");
    let cases: [(&str, f64); 20] = [
        ("2026-02-30", 1_772_409_600_000.0),
        ("Feb 30 2026", 1_772_380_800_000.0),
        ("2026-10-11T07:30+0200", 1_791_696_600_000.0),
        ("Oct 2026", 1_790_784_000_000.0),
        ("2026-10-11", 1_791_676_800_000.0),
        ("2026-10-11T07:30", 1_791_675_000_000.0),
        ("2026-10-11T07:30:05.123Z", 1_791_703_805_123.0),
        ("2026-10-11T07:30:05.1239Z", 1_791_703_805_123.0),
        ("2026-10-11T07:30:05.1Z", 1_791_703_805_100.0),
        ("2026-10-11T07:30:05+02:00", 1_791_696_605_000.0),
        ("+002026-10-11T00:00:00Z", 1_791_676_800_000.0),
        ("2026-10", 1_790_812_800_000.0),
        ("2026-10-11T24:00:00Z", 1_791_763_200_000.0),
        ("Oct 11 2026", 1_791_648_000_000.0),
        ("10/11/2026", 1_791_648_000_000.0),
        (
            "Sun Oct 11 2026 07:30:05 GMT+0800 (China Standard Time)",
            1_791_675_005_000.0,
        ),
        ("Sun, 11 Oct 2026 07:30:05 GMT", 1_791_703_805_000.0),
        ("2026/10/11 07:30", 1_791_675_000_000.0),
        ("11 October 2026 3:04 pm", 1_791_702_240_000.0),
        ("2026-10-11 12:00 am", 1_791_648_000_000.0),
    ];
    for (text, expected) in cases {
        assert_eq!(parse(&mut realm, text), expected, "{text}");
    }
    let zoned: [(&str, f64); 4] = [
        ("Oct 11 2026 07:30 PDT", 7.0),
        ("Oct 11 2026 07:30 EST", 5.0),
        ("Oct 11 2026 07:30 UTC+0530", -5.0),
        ("Oct 11 2026 07:30 GMT-5", 5.0),
    ];
    for (text, hours) in zoned {
        let minutes = if hours == -5.0 { 0.0 } else { 30.0 };
        let hours = if hours == -5.0 { 2.0 } else { 7.0 + hours };
        let expected = utc(&mut realm, &[2026.0, 9.0, 11.0, hours, minutes]);
        assert_eq!(parse(&mut realm, text), expected, "{text}");
    }
    assert_eq!(
        parse(&mut realm, "26 Oct 11 07:30 Z"),
        utc(&mut realm, &[2011.0, 9.0, 26.0, 7.0, 30.0])
    );
    assert_eq!(
        parse(&mut realm, "99 Oct 11"),
        utc(&mut realm, &[1999.0, 9.0, 11.0, -8.0])
    );
    for invalid in [
        "garbage",
        "2026-13-01",
        "2026-02-32",
        "-000000-01-01T00:00:00Z",
        "2026-10-11T07",
        "2026-10-11T07:30:",
        "2026-10-11T07:30:05.",
        "2026-10-11T24:00:01Z",
        "2026-10-11T07:60Z",
        "Oct 11 2026 13:00 pm",
        "Oct",
        "Oct 11 2026 25:00",
        "Oct 40 2026",
        "",
    ] {
        assert!(parse(&mut realm, invalid).is_nan(), "{invalid}");
    }
}

#[test]
fn construction_and_conversion() {
    let mut realm = realm_with(shanghai(), "en-US");
    let original = date(&mut realm, &[n(1000.0)]);
    let copy = date(&mut realm, std::slice::from_ref(&original));
    assert_eq!(call(&mut realm, &copy, "getTime", &[]), "1000");
    let parsed = date(&mut realm, &[s("2026-10-11")]);
    assert_eq!(call(&mut realm, &parsed, "getTime", &[]), "1791676800000");
    let two_digit = date(&mut realm, &[n(99.0), n(0.0)]);
    assert_eq!(call(&mut realm, &two_digit, "getFullYear", &[]), "1999");
    let invalid_year = date(&mut realm, &[n(f64::NAN), n(0.0)]);
    assert_eq!(call(&mut realm, &invalid_year, "getTime", &[]), "NaN");
    let now = date(&mut realm, &[]);
    let current = now.as_object().and_then(time_of).unwrap();
    assert!((current - statics(&mut realm, "now", &[])).abs() < 10_000.0);
    assert!(statics(&mut realm, "UTC", &[]).is_nan());
    assert_eq!(
        statics(&mut realm, "UTC", &[n(2026.0)]),
        1_767_225_600_000.0
    );
    // Calling Date returns the current time as a string.
    let constructor = global(&mut realm, "Date");
    let called = ops::call(&mut realm, &constructor, Value::Undefined, &[]).unwrap();
    assert!(
        ops::to_string(&mut realm, &called)
            .unwrap()
            .to_rust_lossy()
            .contains("GMT+0800")
    );
    // New targets supply the prototype.
    let target = Value::Object(realm.function("T", 0, true, |_, this, _| Ok(this)));
    let made = ops::construct_target(&mut realm, &constructor, &[], &target).unwrap();
    let prototype = ops::get(&mut realm, &target, &Key::name("prototype")).unwrap();
    assert_eq!(
        made.as_object().unwrap().data().prototype.as_ref(),
        prototype.as_object()
    );
    ops::set(&mut realm, &target, &Key::name("prototype"), n(1.0)).unwrap();
    let plain = ops::construct_target(&mut realm, &constructor, &[], &target).unwrap();
    assert_eq!(
        plain.as_object().unwrap().data().prototype,
        Some(realm.date_prototype.clone())
    );
    // Symbol.toPrimitive chooses string or number conversion.
    let key = Key::Symbol(realm.symbols.to_primitive.clone());
    let primitive = ops::get(&mut realm, &original, &key).unwrap();
    let as_number = ops::call(&mut realm, &primitive, original.clone(), &[s("number")]).unwrap();
    assert!(matches!(as_number, Value::Number(1000.0)));
    let as_string = ops::call(&mut realm, &primitive, original.clone(), &[s("default")]).unwrap();
    assert!(matches!(as_string, Value::String(_)));
    for (receiver, hint, message) in [
        (original.clone(), n(1.0), "TypeError: Invalid hint"),
        (
            n(1.0),
            s("number"),
            "TypeError: Date.prototype [ @@toPrimitive ] called on non-object",
        ),
    ] {
        let thrown = ops::call(&mut realm, &primitive, receiver, &[hint]).unwrap_err();
        assert_eq!(
            ops::to_string(&mut realm, &thrown.0)
                .unwrap()
                .to_rust_lossy(),
            message
        );
    }
    // Methods require date receivers; toJSON is generic.
    let get_time = ops::get(&mut realm, &original, &Key::name("getTime")).unwrap();
    let thrown = ops::call(&mut realm, &get_time, n(1.0), &[]).unwrap_err();
    assert_eq!(
        ops::to_string(&mut realm, &thrown.0)
            .unwrap()
            .to_rust_lossy(),
        "TypeError: this is not a Date object. (getTime)"
    );
    let generic = realm.object();
    let iso = realm.function("toISOString", 0, false, |_, _, _| Ok(Value::str("custom")));
    generic.define(Key::name("toISOString"), Property::data(Value::Object(iso)));
    let to_json = ops::get(&mut realm, &original, &Key::name("toJSON")).unwrap();
    let json = ops::call(&mut realm, &to_json, Value::Object(generic), &[]).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &json).unwrap().to_rust_lossy(),
        "custom"
    );
    let inspected = crate::inspect::inspect(&mut realm, &original, 2).unwrap();
    assert_eq!(inspected, "1970-01-01T00:00:01.000Z");
    let debug = format!("{:?}", original.as_object().unwrap().data().kind);
    assert_eq!(debug, "Date(1000)");
}

#[test]
fn setters_replace_components() {
    let mut realm = realm_with(shanghai(), "en-US");
    let base = utc(&mut realm, &[2026.0, 9.0, 11.0, 7.0, 30.0, 5.0, 123.0]);
    let cases: [(&str, &[f64], &str); 16] = [
        ("setMilliseconds", &[7.0], "2026-10-11T07:30:05.007Z"),
        ("setSeconds", &[1.0, 2.0], "2026-10-11T07:30:01.002Z"),
        ("setMinutes", &[1.0], "2026-10-11T07:01:05.123Z"),
        (
            "setHours",
            &[1.0, 2.0, 3.0, 4.0],
            "2026-10-10T17:02:03.004Z",
        ),
        ("setDate", &[1.0], "2026-10-01T07:30:05.123Z"),
        ("setMonth", &[0.0, 2.0], "2026-01-02T07:30:05.123Z"),
        (
            "setFullYear",
            &[2000.0, 1.0, 29.0],
            "2000-02-29T07:30:05.123Z",
        ),
        ("setUTCMilliseconds", &[7.0], "2026-10-11T07:30:05.007Z"),
        ("setUTCSeconds", &[1.0], "2026-10-11T07:30:01.123Z"),
        (
            "setUTCMinutes",
            &[1.0, 2.0, 3.0],
            "2026-10-11T07:01:02.003Z",
        ),
        ("setUTCHours", &[1.0], "2026-10-11T01:30:05.123Z"),
        ("setUTCDate", &[31.0], "2026-10-31T07:30:05.123Z"),
        ("setUTCMonth", &[1.0], "2026-02-11T07:30:05.123Z"),
        ("setUTCFullYear", &[2027.0], "2027-10-11T07:30:05.123Z"),
        ("setTime", &[0.0], "1970-01-01T00:00:00.000Z"),
        ("setYear", &[99.0], "1999-10-11T07:30:05.123Z"),
    ];
    for (method, args, expected) in cases {
        let d = date(&mut realm, &[n(base)]);
        let args: Vec<Value> = args.iter().copied().map(n).collect();
        call(&mut realm, &d, method, &args);
        assert_eq!(
            call(&mut realm, &d, "toISOString", &[]),
            expected,
            "{method}"
        );
    }
    // Invalid dates stay invalid, except setFullYear and setYear start
    // from +0; missing and NaN arguments invalidate.
    let invalid = date(&mut realm, &[n(f64::NAN)]);
    assert_eq!(call(&mut realm, &invalid, "setHours", &[n(1.0)]), "NaN");
    assert_eq!(
        call(&mut realm, &invalid, "setFullYear", &[n(2026.0)]),
        "1767196800000"
    );
    let invalid = date(&mut realm, &[n(f64::NAN)]);
    assert_eq!(
        call(&mut realm, &invalid, "setYear", &[n(2026.0)]),
        "1767196800000"
    );
    let d = date(&mut realm, &[n(base)]);
    assert_eq!(call(&mut realm, &d, "setDate", &[]), "NaN");
    let d = date(&mut realm, &[n(base)]);
    assert_eq!(call(&mut realm, &d, "setYear", &[n(f64::NAN)]), "NaN");
    let d = date(&mut realm, &[n(base)]);
    assert_eq!(call(&mut realm, &d, "setTime", &[]), "NaN");
}

#[test]
fn calendar_arithmetic_round_trips() {
    for year in (-2000..=3000).step_by(7) {
        for (month, date) in [(0, 1), (1, 28), (11, 31)] {
            let year = f64::from(year);
            let time = make_date(make_day(year, f64::from(month), f64::from(date)), 0.0);
            assert_eq!(date_parts(time), (year, month as usize, f64::from(date)));
        }
    }
}

#[test]
fn system_zone_locale_and_conversion_failures() {
    // The Windows zone and user locale, as generated programs use them.
    let mut realm = Realm::new();
    let d = date(&mut realm, &[n(1_791_703_805_123.0)]);
    let offset: f64 = call(&mut realm, &d, "getTimezoneOffset", &[])
        .parse()
        .unwrap();
    assert!(offset.abs() <= 14.0 * 60.0);
    assert!(!call(&mut realm, &d, "toLocaleString", &[]).is_empty());
    // A zone Windows cannot convert with falls back to its standard offset.
    let mut broken = shanghai();
    broken.standard_date.month = 13;
    broken.daylight_date.month = 3;
    let mut realm = realm_with(broken, "en-US");
    let d = date(&mut realm, &[n(0.0)]);
    assert_eq!(call(&mut realm, &d, "getTimezoneOffset", &[]), "-480");
    // An unknown locale falls back to V8's format.
    let mut realm = realm_with(shanghai(), "!");
    let d = date(&mut realm, &[n(0.0)]);
    assert_eq!(
        call(&mut realm, &d, "toLocaleString", &[]),
        "Thu Jan 01 1970 08:00:00"
    );
    // Methods need dates; inspection shows invalid and subclassed dates.
    let plain = Value::Object(realm.object());
    assert_eq!(call(&mut realm, &plain, "toString", &[]), "[object Object]");
    let get_time = ops::get(&mut realm, &d, &Key::name("getTime")).unwrap();
    let thrown = ops::call(&mut realm, &get_time, plain, &[]).unwrap_err();
    assert!(
        ops::to_string(&mut realm, &thrown.0)
            .unwrap()
            .to_rust_lossy()
            .contains("getTime")
    );
    let invalid = date(&mut realm, &[n(f64::NAN)]);
    assert_eq!(
        crate::inspect::inspect(&mut realm, &invalid, 2).unwrap(),
        "Invalid Date"
    );
    let class = realm.function("Stamp", 0, true, |_, this, _| Ok(this));
    let prototype = Object::new(Some(realm.date_prototype.clone()), Kind::Ordinary);
    prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(class.clone())),
    );
    class.define(
        Key::name("prototype"),
        Property::hidden(Value::Object(prototype.clone())),
    );
    let stamped = Object::new(Some(prototype), Kind::Date(0.0));
    stamped.define(Key::name("x"), Property::data(n(1.0)));
    assert_eq!(
        crate::inspect::inspect(&mut realm, &Value::Object(stamped), 2).unwrap(),
        "Stamp 1970-01-01T00:00:00.000Z { x: 1 }"
    );
}
