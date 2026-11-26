use super::*;

/// Evaluates `new Intl[name](locales, options)[method](args)` and returns
/// the result as text.
fn call(
    realm: &mut Realm,
    name: &str,
    locales: Value,
    options: &[(&str, Value)],
    method: &str,
    args: &[Value],
) -> String {
    let global = Value::Object(realm.global_object());
    let intl = ops::get(realm, &global, &Key::name("Intl")).unwrap();
    let constructor = ops::get(realm, &intl, &Key::name(name)).unwrap();
    let option_object = realm.object();
    for (key, value) in options {
        option_object.define(Key::name(key), Property::data(value.clone()));
    }
    let instance = ops::construct(
        realm,
        &constructor,
        &[locales, Value::Object(option_object)],
    )
    .unwrap();
    let function = ops::get(realm, &instance, &Key::name(method)).unwrap();
    let result = ops::call(realm, &function, instance, args).unwrap();
    ops::to_string(realm, &result).unwrap().to_rust_lossy()
}

fn segments(realm: &mut Realm, granularity: &str, text: &str) -> Vec<String> {
    let global = Value::Object(realm.global_object());
    let intl = ops::get(realm, &global, &Key::name("Intl")).unwrap();
    let constructor = ops::get(realm, &intl, &Key::name("Segmenter")).unwrap();
    let options = realm.object();
    options.define(
        Key::name("granularity"),
        Property::data(Value::str(granularity)),
    );
    let segmenter = ops::construct(
        realm,
        &constructor,
        &[Value::str("en"), Value::Object(options)],
    )
    .unwrap();
    let segment = ops::get(realm, &segmenter, &Key::name("segment")).unwrap();
    let segments = ops::call(realm, &segment, segmenter, &[Value::str(text)]).unwrap();
    crate::iter::collect(realm, &segments)
        .unwrap()
        .into_iter()
        .map(|record| {
            let text = ops::get(realm, &record, &Key::name("segment")).unwrap();
            ops::to_string(realm, &text).unwrap().to_rust_lossy()
        })
        .collect()
}

#[test]
fn segmenters_split_grapheme_clusters_words_and_sentences() {
    let mut realm = Realm::new();
    let graphemes = segments(&mut realm, "grapheme", "e\u{301}👨\u{200d}👩\u{200d}👧a");
    if icu().is_some() {
        assert_eq!(graphemes, ["e\u{301}", "👨\u{200d}👩\u{200d}👧", "a"]);
        assert_eq!(
            segments(&mut realm, "word", "It's 3.5!"),
            ["It's", " ", "3.5", "!"]
        );
        assert_eq!(
            segments(&mut realm, "sentence", "One. Two?"),
            ["One. ", "Two?"]
        );
    } else {
        // Without ICU, code points.
        assert_eq!(graphemes.len(), 8);
    }
    assert!(segments(&mut realm, "grapheme", "").is_empty());
}

#[test]
fn formatters_follow_the_locale_and_options() {
    if icu().is_none() {
        return;
    }
    let mut realm = Realm::new();
    let number = |realm: &mut Realm, locale: &str, options: &[(&str, Value)], value: f64| {
        call(
            realm,
            "NumberFormat",
            Value::str(locale),
            options,
            "format",
            &[Value::Number(value)],
        )
    };
    assert_eq!(
        number(&mut realm, "en-US", &[], 1234567.891),
        "1,234,567.891"
    );
    assert_eq!(
        number(&mut realm, "de-DE", &[], 1234567.891),
        "1.234.567,891"
    );
    assert_eq!(
        number(
            &mut realm,
            "en-US",
            &[
                ("style", Value::str("currency")),
                ("currency", Value::str("EUR"))
            ],
            -5.5
        ),
        "-€5.50"
    );
    assert_eq!(
        number(
            &mut realm,
            "en-US",
            &[("style", Value::str("percent"))],
            0.256
        ),
        "26%"
    );
    assert_eq!(
        number(
            &mut realm,
            "en-US",
            &[
                ("style", Value::str("unit")),
                ("unit", Value::str("kilometer-per-hour"))
            ],
            50.0
        ),
        "50 km/h"
    );
    assert_eq!(
        number(
            &mut realm,
            "en-US",
            &[("notation", Value::str("compact"))],
            12345.0
        ),
        "12K"
    );
    let date = |realm: &mut Realm, locale: &str, options: &[(&str, Value)]| {
        let mut options = options.to_vec();
        options.push(("timeZone", Value::str("UTC")));
        call(
            realm,
            "DateTimeFormat",
            Value::str(locale),
            &options,
            "format",
            &[Value::Number(1709647629000.0)],
        )
    };
    assert_eq!(date(&mut realm, "en-US", &[]), "3/5/2024");
    assert_eq!(date(&mut realm, "de-DE", &[]), "5.3.2024");
    assert_eq!(
        date(&mut realm, "en-US", &[("dateStyle", Value::str("full"))]),
        "Tuesday, March 5, 2024"
    );
    // Hour cycles CLDR changed after Windows' ICU 64 follow Chromium.
    let hours = [
        ("hour", Value::str("numeric")),
        ("minute", Value::str("2-digit")),
    ];
    let seconds = [
        ("hour", Value::str("numeric")),
        ("minute", Value::str("2-digit")),
        ("second", Value::str("2-digit")),
    ];
    for (locale, options, expected) in [
        ("zh-CN", &seconds[..], "14:07:09"),
        ("zh-CN", &[("timeStyle", Value::str("short"))], "14:07"),
        (
            "zh-CN",
            &[
                ("dateStyle", Value::str("medium")),
                ("timeStyle", Value::str("short")),
            ],
            "2024年3月5日 14:07",
        ),
        ("zh", &[("timeStyle", Value::str("medium"))], "14:07:09"),
        (
            "zh-CN",
            &[
                ("hour", Value::str("numeric")),
                ("hour12", Value::Boolean(true)),
            ],
            "下午2时",
        ),
        ("zh-TW", &hours, "下午2:07"),
        // The day period's text is Windows' CLDR's ("p.m." in Chromium).
        (
            "es-MX",
            &[("timeStyle", Value::str("medium"))],
            "2:07:09 p.\u{a0}m.",
        ),
        ("es-MX", &hours, "2:07 p.\u{a0}m."),
    ] {
        // Newer Windows ICU (CLDR 42 and later) puts a narrow no-break space
        // before the day period where older versions put a space.
        let formatted = date(&mut realm, locale, options).replace('\u{202f}', " ");
        assert_eq!(formatted, expected, "{locale}");
    }
    assert_eq!(
        call(
            &mut realm,
            "PluralRules",
            Value::str("en"),
            &[("type", Value::str("ordinal"))],
            "select",
            &[Value::Number(2.0)]
        ),
        "two"
    );
    assert_eq!(
        call(
            &mut realm,
            "RelativeTimeFormat",
            Value::str("en"),
            &[("numeric", Value::str("auto"))],
            "format",
            &[Value::Number(-1.0), Value::str("day")]
        ),
        "yesterday"
    );
    assert_eq!(
        call(
            &mut realm,
            "Collator",
            Value::str("en"),
            &[("numeric", Value::Boolean(true))],
            "compare",
            &[Value::str("a10"), Value::str("a2")]
        ),
        "1"
    );
    let list = Value::Object(realm.array(vec![Value::str("a"), Value::str("b"), Value::str("c")]));
    assert_eq!(
        call(
            &mut realm,
            "ListFormat",
            Value::str("en"),
            &[],
            "format",
            &[list]
        ),
        "a, b, and c"
    );
}

#[test]
fn options_are_validated() {
    let mut realm = Realm::new();
    let global = Value::Object(realm.global_object());
    let intl = ops::get(&mut realm, &global, &Key::name("Intl")).unwrap();
    let constructor = ops::get(&mut realm, &intl, &Key::name("NumberFormat")).unwrap();
    let options = realm.object();
    options.define(Key::name("style"), Property::data(Value::str("currency")));
    assert!(
        ops::construct(
            &mut realm,
            &constructor,
            &[Value::Undefined, Value::Object(options)]
        )
        .is_err()
    );
    let options = realm.object();
    options.define(Key::name("style"), Property::data(Value::str("bogus")));
    assert!(
        ops::construct(
            &mut realm,
            &constructor,
            &[Value::Undefined, Value::Object(options)]
        )
        .is_err()
    );
    assert!(ops::construct(&mut realm, &constructor, &[Value::str("not a tag!")]).is_err());
}

#[test]
fn display_names_name_codes_as_chromium_does() {
    let mut realm = Realm::new();
    let mut of = |locale: &str, options: &[(&str, &str)], code: &str| {
        let options: Vec<(&str, Value)> = options
            .iter()
            .map(|(key, value)| (*key, Value::str(value)))
            .collect();
        call(
            &mut realm,
            "DisplayNames",
            Value::str(locale),
            &options,
            "of",
            &[Value::str(code)],
        )
    };
    let language = [("type", "language")];
    // Expected values are V8's (Node 22 with full ICU).
    assert_eq!(of("en", &language, "en"), "English");
    assert_eq!(of("en", &language, "en-US"), "American English");
    assert_eq!(
        of(
            "en",
            &[("type", "language"), ("languageDisplay", "standard")],
            "en-US"
        ),
        "English (United States)"
    );
    assert_eq!(of("en", &language, "zh-Hant"), "Traditional Chinese");
    assert_eq!(of("fr", &language, "en"), "anglais");
    assert_eq!(of("zh-CN", &language, "ja"), "日语");
    assert_eq!(of("en", &language, "xx"), "xx");
    assert_eq!(
        of("en", &[("type", "language"), ("fallback", "none")], "xx"),
        "undefined"
    );
    assert_eq!(of("en", &[("type", "region")], "419"), "Latin America");
    assert_eq!(of("en", &[("type", "region")], "us"), "us");
    assert_eq!(of("en", &[("type", "script")], "Latn"), "Latin");
    assert_eq!(of("en", &[("type", "currency")], "usd"), "US Dollar");
    assert_eq!(of("en", &[("type", "currency")], "XYZ"), "XYZ");
    assert_eq!(
        of("en", &[("type", "currency"), ("fallback", "none")], "XYZ"),
        "undefined"
    );
    assert_eq!(
        of("en", &[("type", "calendar")], "gregory"),
        "Gregorian Calendar"
    );
    assert_eq!(
        of("en", &[("type", "calendar")], "buddhist"),
        "Buddhist Calendar"
    );
    assert_eq!(
        of("en", &[("type", "dateTimeField")], "weekday"),
        "day of the week"
    );
    assert_eq!(
        of(
            "en",
            &[("type", "dateTimeField"), ("style", "short")],
            "month"
        ),
        "mo."
    );
}

#[test]
fn display_names_check_their_type_and_codes() {
    let mut realm = Realm::new();
    let global = Value::Object(realm.global_object());
    let intl = ops::get(&mut realm, &global, &Key::name("Intl")).unwrap();
    let constructor = ops::get(&mut realm, &intl, &Key::name("DisplayNames")).unwrap();
    assert!(ops::construct(&mut realm, &constructor, &[Value::str("en")]).is_err());
    for (kind, code) in [
        ("region", "u"),
        ("script", "Lat"),
        ("currency", "US"),
        ("dateTimeField", "nope"),
        ("language", "en-u-ca-roc"),
    ] {
        let options = realm.object();
        options.define(Key::name("type"), Property::data(Value::str(kind)));
        let names = ops::construct(
            &mut realm,
            &constructor,
            &[Value::str("en"), Value::Object(options)],
        )
        .unwrap();
        let of = ops::get(&mut realm, &names, &Key::name("of")).unwrap();
        assert!(
            ops::call(&mut realm, &of, names, &[Value::str(code)]).is_err(),
            "{kind} {code}"
        );
    }
}

/// `new Intl.Locale(tag, options)`.
fn locale(realm: &mut Realm, tag: &str, options: &[(&str, Value)]) -> Result<Value, Thrown> {
    let global = Value::Object(realm.global_object());
    let intl = ops::get(realm, &global, &Key::name("Intl")).unwrap();
    let constructor = ops::get(realm, &intl, &Key::name("Locale")).unwrap();
    let option_object = realm.object();
    for (key, value) in options {
        option_object.define(Key::name(key), Property::data(value.clone()));
    }
    ops::construct(
        realm,
        &constructor,
        &[Value::str(tag), Value::Object(option_object)],
    )
}

/// A property of a value, or the result of calling its method, as text.
fn text_of(realm: &mut Realm, value: &Value, path: &str) -> String {
    let mut value = value.clone();
    for name in path.split('.') {
        value = match name.strip_suffix("()") {
            Some(method) => {
                let function = ops::get(realm, &value, &Key::name(method)).unwrap();
                ops::call(realm, &function, value, &[]).unwrap()
            }
            None => ops::get(realm, &value, &Key::name(name)).unwrap(),
        };
    }
    ops::to_string(realm, &value).unwrap().to_rust_lossy()
}

#[test]
fn locales_parse_canonicalize_and_take_options_as_v8_does() {
    let mut realm = Realm::new();
    for (tag, expected) in [
        ("EN-latn-us", "en-Latn-US"),
        ("iw", "he"),
        ("en-u-kn-true", "en-u-kn"),
        ("en-u-ca-gregory-co-phonebk", "en-u-ca-gregory-co-phonebk"),
        ("de-1996-u-ca-buddhist", "de-1996-u-ca-buddhist"),
        ("en-x-private", "en-x-private"),
        ("en-u-ca", "en-u-ca"),
        (
            "zh-hant-tw-u-nu-hanidec-ca-chinese",
            "zh-Hant-TW-u-ca-chinese-nu-hanidec",
        ),
    ] {
        let value = locale(&mut realm, tag, &[]).unwrap();
        assert_eq!(text_of(&mut realm, &value, "toString()"), expected, "{tag}");
    }
    let value = locale(
        &mut realm,
        "zh-hant-tw-u-ca-chinese-hc-h12-kn",
        &[
            ("region", Value::str("HK")),
            ("numeric", Value::Boolean(false)),
        ],
    )
    .unwrap();
    assert_eq!(
        text_of(&mut realm, &value, "toString()"),
        "zh-Hant-HK-u-ca-chinese-hc-h12-kn-false"
    );
    for (field, expected) in [
        ("baseName", "zh-Hant-HK"),
        ("language", "zh"),
        ("script", "Hant"),
        ("region", "HK"),
        ("calendar", "chinese"),
        ("hourCycle", "h12"),
        ("numeric", "false"),
        ("caseFirst", "undefined"),
        ("collation", "undefined"),
        ("numberingSystem", "undefined"),
    ] {
        assert_eq!(text_of(&mut realm, &value, field), expected, "{field}");
    }
    let value = locale(
        &mut realm,
        "en-us",
        &[
            ("language", Value::str("FR")),
            ("script", Value::str("cyrl")),
            ("numeric", Value::Boolean(true)),
            ("caseFirst", Value::str("lower")),
            ("numberingSystem", Value::str("arab")),
            ("collation", Value::str("emoji")),
        ],
    )
    .unwrap();
    assert_eq!(
        text_of(&mut realm, &value, "toString()"),
        "fr-Cyrl-US-u-co-emoji-kf-lower-kn-nu-arab"
    );
    assert_eq!(text_of(&mut realm, &value, "numeric"), "true");
    for (tag, options) in [
        ("x", &[][..]),
        ("en-u-ca-gregorian", &[]),
        ("en-GB-oed", &[]),
        ("en-a-x", &[]),
        ("de-1996-1996", &[]),
        ("en", &[("region", Value::str("1"))]),
        ("en", &[("calendar", Value::str("a"))]),
        ("en", &[("hourCycle", Value::str("h13"))]),
    ] {
        let error = locale(&mut realm, tag, options).unwrap_err();
        assert!(
            ops::to_string(&mut realm, &error.0)
                .unwrap()
                .to_rust_lossy()
                .starts_with("RangeError"),
            "{tag}"
        );
    }
    let global = Value::Object(realm.global_object());
    let intl = ops::get(&mut realm, &global, &Key::name("Intl")).unwrap();
    let constructor = ops::get(&mut realm, &intl, &Key::name("Locale")).unwrap();
    assert!(
        ops::call(
            &mut realm,
            &constructor,
            Value::Undefined,
            &[Value::str("en")]
        )
        .is_err()
    );
    assert!(ops::construct(&mut realm, &constructor, &[]).is_err());
    // A locale gives its tag to another, and other Intl objects take it.
    let english = locale(&mut realm, "en", &[]).unwrap();
    let copy = ops::construct(&mut realm, &constructor, core::slice::from_ref(&english)).unwrap();
    assert_eq!(text_of(&mut realm, &copy, "toString()"), "en");
    let german = locale(&mut realm, "de", &[]).unwrap();
    assert_eq!(
        call(
            &mut realm,
            "NumberFormat",
            german,
            &[],
            "format",
            &[Value::Number(1234.5)]
        ),
        "1.234,5"
    );
    let key = Key::Symbol(realm.symbols.to_string_tag.clone());
    let tag = ops::get(&mut realm, &english, &key).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &tag).unwrap().to_rust_lossy(),
        "Intl.Locale"
    );
}

#[test]
fn locales_maximize_minimize_and_describe_themselves_with_windows_icu() {
    let mut realm = Realm::new();
    for (tag, method, expected) in [
        ("en", "maximize()", "en-Latn-US"),
        ("und", "maximize()", "en-Latn-US"),
        ("sr", "maximize()", "sr-Cyrl-RS"),
        ("zh-TW", "maximize()", "zh-Hant-TW"),
        ("zh-Hans-CN", "minimize()", "zh"),
        ("en-Latn-US-u-ca-gregory", "minimize()", "en-u-ca-gregory"),
    ] {
        let value = locale(&mut realm, tag, &[]).unwrap();
        assert_eq!(
            text_of(&mut realm, &value, &format!("{method}.toString()")),
            expected,
            "{tag}"
        );
    }
    for (tag, direction, first, weekend, minimal, cycle, calendars, system) in [
        ("en-US", "ltr", "7", "6,7", "1", "h12", "gregory", "latn"),
        (
            "ar-EG",
            "rtl",
            "6",
            "5,6",
            "1",
            "h12",
            "gregory,coptic,islamic,islamic-civil,islamic-tbla",
            "arab",
        ),
        ("de", "ltr", "1", "6,7", "4", "h23", "gregory", "latn"),
        (
            "zh-CN",
            "ltr",
            "1",
            "6,7",
            "1",
            "h23",
            "gregory,chinese",
            "latn",
        ),
        (
            "he-IL",
            "rtl",
            "7",
            "5,6",
            "1",
            "h23",
            "gregory,hebrew,islamic,islamic-civil,islamic-tbla",
            "latn",
        ),
        (
            "en-u-fw-mon-hc-h11-ca-japanese-nu-thai",
            "ltr",
            "1",
            "6,7",
            "1",
            "h11",
            "japanese",
            "thai",
        ),
    ] {
        let value = locale(&mut realm, tag, &[]).unwrap();
        for (path, expected) in [
            ("getTextInfo().direction", direction),
            ("textInfo.direction", direction),
            ("getWeekInfo().firstDay", first),
            ("getWeekInfo().weekend", weekend),
            ("weekInfo.minimalDays", minimal),
            ("getHourCycles()", cycle),
            ("getCalendars()", calendars),
            ("getNumberingSystems()", system),
        ] {
            assert_eq!(text_of(&mut realm, &value, path), expected, "{tag} {path}");
        }
    }
    let german = locale(&mut realm, "de", &[]).unwrap();
    assert_eq!(
        text_of(&mut realm, &german, "getCollations()"),
        "emoji,eor,phonebk"
    );
    assert_eq!(text_of(&mut realm, &german, "getTimeZones()"), "undefined");
    let british = locale(&mut realm, "en-GB", &[]).unwrap();
    assert_eq!(text_of(&mut realm, &british, "timeZones"), "Europe/London");
    let chinese = locale(&mut realm, "zh-CN", &[]).unwrap();
    // Windows ICU versions differ in listing Urumqi's zone for China.
    let zones = text_of(&mut realm, &chinese, "getTimeZones()");
    assert!(
        ["Asia/Shanghai", "Asia/Shanghai,Asia/Urumqi"].contains(&zones.as_str()),
        "{zones}"
    );
}
