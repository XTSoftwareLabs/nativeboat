use super::*;
use crate::object::Slot;

/// The text of a result computed with the realm.
macro_rules! r {
    ($realm:ident, $value:expr) => {{
        let value = $value;
        rust(&mut $realm, value)
    }};
}

/// The parts of a list result computed with the realm.
macro_rules! p {
    ($realm:ident, $value:expr) => {{
        let value = $value;
        parts(&mut $realm, value)
    }};
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

/// Calls `String.prototype[name]` on a receiver.
fn call(realm: &mut Realm, this: &Value, name: &str, args: &[Value]) -> Completion {
    let prototype = Value::Object(realm.string_prototype.clone());
    let method = ops::get(realm, &prototype, &Key::name(name)).unwrap();
    ops::call(realm, &method, this.clone(), args)
}

fn statics(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let constructor = global(realm, "String");
    let function = ops::get(realm, &constructor, &Key::name(name)).unwrap();
    ops::call(realm, &function, Value::Undefined, args)
}

/// The text of a string result.
fn rust(realm: &mut Realm, value: Completion) -> String {
    let value = value.unwrap();
    ops::to_string(realm, &value).unwrap().to_rust_lossy()
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

fn number(value: Completion) -> f64 {
    match value.unwrap() {
        Value::Number(number) => number,
        other => panic!("{other:?}"),
    }
}

fn boolean(value: Completion) -> bool {
    match value.unwrap() {
        Value::Boolean(flag) => flag,
        other => panic!("{other:?}"),
    }
}

fn function(
    realm: &Realm,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Value {
    Value::Object(realm.function("f", 0, false, code))
}

/// A list result as `a|b|c`.
fn parts(realm: &mut Realm, value: Completion) -> String {
    let value = value.unwrap();
    let length = crate::array_builtin::length_of(realm, &value).unwrap();
    (0..length)
        .map(|index| {
            let part = ops::get(realm, &value, &index_key(index)).unwrap();
            ops::to_string(realm, &part).unwrap().to_rust_lossy()
        })
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn the_constructor_converts_wraps_and_builds_strings() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "String");
    assert_eq!(
        r!(
            realm,
            ops::call(&mut realm, &constructor, Value::Undefined, &[])
        ),
        ""
    );
    assert_eq!(
        r!(
            realm,
            ops::call(&mut realm, &constructor, Value::Undefined, &[n(4.5)])
        ),
        "4.5"
    );
    let symbol = Value::Symbol(realm.symbols.iterator.clone());
    let described = ops::call(
        &mut realm,
        &constructor,
        Value::Undefined,
        std::slice::from_ref(&symbol),
    );
    assert_eq!(r!(realm, described), "Symbol(Symbol.iterator)");
    let refused = ops::construct(&mut realm, &constructor, &[symbol]);
    assert!(message(&mut realm, refused).starts_with("TypeError"));
    let wrapper = ops::construct(&mut realm, &constructor, &[s("abc")]).unwrap();
    assert_eq!(r!(realm, call(&mut realm, &wrapper, "valueOf", &[])), "abc");
    let length = ops::get(&mut realm, &wrapper, &Key::name("length")).unwrap();
    assert!(ops::strict_equals(&length, &n(3.0)));
    // Subclass instances take the new target's prototype.
    let target = realm.function("Target", 0, true, |_, _, _| Ok(Value::Undefined));
    let custom = realm.object();
    target.define(
        Key::name("prototype"),
        Property::data(Value::Object(custom.clone())),
    );
    let instance = ops::construct_target(
        &mut realm,
        &constructor,
        &[],
        &Value::Object(target.clone()),
    )
    .unwrap();
    assert_eq!(instance.as_object().unwrap().data().prototype, Some(custom));
    target.define(Key::name("prototype"), Property::data(n(1.0)));
    let fallback =
        ops::construct_target(&mut realm, &constructor, &[], &Value::Object(target)).unwrap();
    assert_eq!(
        fallback.as_object().unwrap().data().prototype,
        Some(realm.string_prototype.clone())
    );
    assert_eq!(
        r!(realm, call(&mut realm, &s("abc"), "toString", &[])),
        "abc"
    );
    // String.prototype is itself the empty String object.
    let prototype = Value::Object(realm.string_prototype.clone());
    assert_eq!(r!(realm, call(&mut realm, &prototype, "toString", &[])), "");
    for receiver in [n(1.0), Value::Object(realm.object())] {
        let refused = call(&mut realm, &receiver, "toString", &[]);
        assert!(message(&mut realm, refused).contains("requires a string"));
    }
    assert_eq!(
        r!(
            realm,
            statics(&mut realm, "fromCharCode", &[n(72.0), n(65_641.0)])
        ),
        "Hi"
    );
    assert_eq!(
        r!(
            realm,
            statics(&mut realm, "fromCodePoint", &[n(0x1F600 as f64), n(65.0)])
        ),
        "😀A"
    );
    for invalid in [n(-1.0), n(1.5), n(1_114_112.0), n(f64::NAN)] {
        let refused = statics(&mut realm, "fromCodePoint", &[invalid]);
        assert!(message(&mut realm, refused).starts_with("RangeError: Invalid code point"));
    }
    let template = realm.object();
    let literals = realm.array(vec![s("a"), s("b"), s("c")]);
    template.define(Key::name("raw"), Property::data(Value::Object(literals)));
    let template = Value::Object(template);
    let raw = statics(&mut realm, "raw", &[template.clone(), n(1.0)]);
    assert_eq!(r!(realm, raw), "a1bc");
    let empty = realm.object();
    empty.define(
        Key::name("raw"),
        Property::data(Value::Object(realm.array(vec![]))),
    );
    assert_eq!(
        r!(realm, statics(&mut realm, "raw", &[Value::Object(empty)])),
        ""
    );
    let thrown = call(&mut realm, &Value::Undefined, "trim", &[]);
    assert!(message(&mut realm, thrown).contains("called on null or undefined"));
}

#[test]
fn indexing_and_searching_use_code_units() {
    let mut realm = Realm::new();
    let text = s("a😀bcab");
    assert_eq!(r!(realm, call(&mut realm, &text, "at", &[n(-1.0)])), "b");
    assert!(matches!(
        call(&mut realm, &text, "at", &[n(9.0)]).unwrap(),
        Value::Undefined
    ));
    assert!(matches!(
        call(&mut realm, &text, "at", &[n(-9.0)]).unwrap(),
        Value::Undefined
    ));
    assert_eq!(r!(realm, call(&mut realm, &text, "charAt", &[n(0.0)])), "a");
    assert_eq!(r!(realm, call(&mut realm, &text, "charAt", &[n(99.0)])), "");
    assert_eq!(
        number(call(&mut realm, &text, "charCodeAt", &[n(1.0)])),
        55_357.0
    );
    assert!(number(call(&mut realm, &text, "charCodeAt", &[n(-1.0)])).is_nan());
    assert_eq!(
        number(call(&mut realm, &text, "codePointAt", &[n(1.0)])),
        128_512.0
    );
    assert_eq!(
        number(call(&mut realm, &text, "codePointAt", &[n(2.0)])),
        56_832.0
    );
    assert_eq!(
        number(call(&mut realm, &text, "codePointAt", &[n(6.0)])),
        98.0
    );
    assert!(matches!(
        call(&mut realm, &text, "codePointAt", &[n(7.0)]).unwrap(),
        Value::Undefined
    ));
    assert_eq!(number(call(&mut realm, &text, "indexOf", &[s("b")])), 3.0);
    assert_eq!(
        number(call(&mut realm, &text, "indexOf", &[s("b"), n(4.0)])),
        6.0
    );
    assert_eq!(number(call(&mut realm, &text, "indexOf", &[s("")])), 0.0);
    assert_eq!(
        number(call(&mut realm, &text, "indexOf", &[s(""), n(99.0)])),
        7.0
    );
    assert_eq!(number(call(&mut realm, &text, "indexOf", &[s("zz")])), -1.0);
    assert_eq!(
        number(call(&mut realm, &s("ab"), "indexOf", &[s("abc")])),
        -1.0
    );
    assert_eq!(
        number(call(&mut realm, &text, "lastIndexOf", &[s("a")])),
        5.0
    );
    assert_eq!(
        number(call(&mut realm, &text, "lastIndexOf", &[s("a"), n(4.0)])),
        0.0
    );
    assert_eq!(
        number(call(&mut realm, &text, "lastIndexOf", &[s("a"), n(-5.0)])),
        0.0
    );
    assert_eq!(
        number(call(&mut realm, &text, "lastIndexOf", &[s("q")])),
        -1.0
    );
    assert_eq!(
        number(call(&mut realm, &s("a"), "lastIndexOf", &[s("abc")])),
        -1.0
    );
    assert!(boolean(call(&mut realm, &text, "includes", &[s("ca")])));
    assert!(!boolean(call(
        &mut realm,
        &text,
        "includes",
        &[s("a"), n(6.0)]
    )));
    assert!(boolean(call(
        &mut realm,
        &text,
        "startsWith",
        &[s("bc"), n(3.0)]
    )));
    assert!(boolean(call(
        &mut realm,
        &text,
        "endsWith",
        &[s("ca"), n(6.0)]
    )));
    assert!(boolean(call(&mut realm, &text, "endsWith", &[s("ab")])));
    // Arguments that claim to be regular expressions are rejected.
    let pattern = realm.object();
    pattern.define(
        Key::Symbol(realm.symbols.match_.clone()),
        Property::data(Value::Boolean(true)),
    );
    let pattern = Value::Object(pattern);
    for name in ["includes", "startsWith", "endsWith"] {
        let refused = call(&mut realm, &text, name, std::slice::from_ref(&pattern));
        assert!(
            message(&mut realm, refused).contains("must not be a regular expression"),
            "{name}"
        );
    }
    let plain = realm.object();
    plain.define(
        Key::Symbol(realm.symbols.match_.clone()),
        Property::data(Value::Boolean(false)),
    );
    assert!(!boolean(call(
        &mut realm,
        &text,
        "includes",
        &[Value::Object(plain)]
    )));
    let object = Value::Object(realm.object());
    let found = call(&mut realm, &s("x[object Object]"), "includes", &[object]);
    assert!(boolean(found));
    assert!(boolean(call(&mut realm, &s("ok"), "isWellFormed", &[])));
    let lone = Value::String(JsStr::from_units(&[0x61, 0xD800, 0xDC00, 0xDC00]));
    assert!(!boolean(call(&mut realm, &lone, "isWellFormed", &[])));
    let repaired = call(&mut realm, &lone, "toWellFormed", &[]).unwrap();
    let Value::String(repaired) = repaired else {
        panic!()
    };
    assert_eq!(repaired.units(), [0x61, 0xD800, 0xDC00, 0xFFFD]);
}

#[test]
fn slicing_padding_and_trimming() {
    let mut realm = Realm::new();
    let text = s("abcdef");
    assert_eq!(
        r!(realm, call(&mut realm, &text, "slice", &[n(-4.0), n(-1.0)])),
        "cde"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &text, "slice", &[n(4.0), n(1.0)])),
        ""
    );
    assert_eq!(
        r!(realm, call(&mut realm, &text, "slice", &[n(-99.0)])),
        "abcdef"
    );
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &text, "substring", &[n(4.0), n(1.0)])
        ),
        "bcd"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &text, "substring", &[n(2.0)])),
        "cdef"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &text, "substr", &[n(-3.0), n(2.0)])),
        "de"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &text, "substr", &[n(1.0)])),
        "bcdef"
    );
    assert_eq!(r!(realm, call(&mut realm, &text, "substr", &[n(9.0)])), "");
    assert_eq!(
        r!(realm, call(&mut realm, &text, "substr", &[n(2.0), n(-1.0)])),
        ""
    );
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &s("5"), "padStart", &[n(3.0), s("0")])
        ),
        "005"
    );
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &s("5"), "padEnd", &[n(4.0), s("ab")])
        ),
        "5aba"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &s("5"), "padEnd", &[n(3.0)])),
        "5  "
    );
    assert_eq!(
        r!(realm, call(&mut realm, &s("long"), "padStart", &[n(2.0)])),
        "long"
    );
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &s("x"), "padStart", &[n(5.0), s("")])
        ),
        "x"
    );
    let huge = call(&mut realm, &s("x"), "padStart", &[n(1e10)]);
    assert_eq!(
        message(&mut realm, huge),
        "RangeError: Invalid string length"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &s("ab"), "repeat", &[n(3.0)])),
        "ababab"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &s("ab"), "repeat", &[n(0.0)])),
        ""
    );
    assert_eq!(r!(realm, call(&mut realm, &s(""), "repeat", &[n(5.0)])), "");
    for count in [-1.0, f64::INFINITY] {
        let refused = call(&mut realm, &s("a"), "repeat", &[n(count)]);
        assert_eq!(
            message(&mut realm, refused),
            "RangeError: Invalid count value"
        );
    }
    let huge = call(&mut realm, &s("ab"), "repeat", &[n(1e9)]);
    assert_eq!(
        message(&mut realm, huge),
        "RangeError: Invalid string length"
    );
    let spaced = s("\u{FEFF}\t x y \u{3000}\n");
    assert_eq!(r!(realm, call(&mut realm, &spaced, "trim", &[])), "x y");
    assert_eq!(
        r!(realm, call(&mut realm, &spaced, "trimStart", &[])),
        "x y \u{3000}\n"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &spaced, "trimEnd", &[])),
        "\u{FEFF}\t x y"
    );
    assert_eq!(r!(realm, call(&mut realm, &s("   "), "trimEnd", &[])), "");
    let prototype = Value::Object(realm.string_prototype.clone());
    let left = ops::get(&mut realm, &prototype, &Key::name("trimLeft")).unwrap();
    let start = ops::get(&mut realm, &prototype, &Key::name("trimStart")).unwrap();
    assert!(ops::strict_equals(&left, &start));
    let joined = call(&mut realm, &s("a"), "concat", &[n(1.0), Value::Null]);
    assert_eq!(r!(realm, joined), "a1null");
}

#[test]
fn case_conversion_is_full_unicode_with_final_sigma() {
    let mut realm = Realm::new();
    let lower = call(&mut realm, &s("ΟΔΟΣ ΣΑΣ. Σ ÄÖ İ"), "toLowerCase", &[]);
    assert_eq!(r!(realm, lower), "οδος σας. σ äö i\u{307}");
    let after_mark = call(&mut realm, &s("AΣ\u{301}"), "toLowerCase", &[]);
    assert_eq!(r!(realm, after_mark), "aς\u{301}");
    let before_letter = call(&mut realm, &s("ΣA"), "toLocaleLowerCase", &[]);
    assert_eq!(r!(realm, before_letter), "σa");
    let upper = call(&mut realm, &s("straße ǆ"), "toUpperCase", &[]);
    assert_eq!(r!(realm, upper), "STRASSE Ǆ");
    let locale = call(&mut realm, &s("ab"), "toLocaleUpperCase", &[]);
    assert_eq!(r!(realm, locale), "AB");
    let lone = Value::String(JsStr::from_units(&[0xD800, 0x41]));
    let converted = call(&mut realm, &lone, "toLowerCase", &[]).unwrap();
    let Value::String(converted) = converted else {
        panic!()
    };
    assert_eq!(converted.units(), [0xD800, 0x61]);
}

#[test]
fn normalization_forms_are_validated_and_applied() {
    let mut realm = Realm::new();
    let composed = s("é");
    let decomposed = call(&mut realm, &composed, "normalize", &[s("NFD")]).unwrap();
    let Value::String(units) = &decomposed else {
        panic!()
    };
    assert_eq!(units.units(), [0x65, 0x301]);
    let back = call(&mut realm, &decomposed, "normalize", &[]);
    assert_eq!(r!(realm, back), "é");
    assert_eq!(
        r!(realm, call(&mut realm, &s("ﬁ"), "normalize", &[s("NFKC")])),
        "fi"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &s("ﬁ"), "normalize", &[s("NFKD")])),
        "fi"
    );
    let refused = call(&mut realm, &composed, "normalize", &[s("nfc")]);
    assert!(message(&mut realm, refused).starts_with("RangeError: The normalization form"));
}

/// Compares with the root collation used without ICU.
fn compare(realm: &mut Realm, left: &str, right: &str, options: Option<Value>) -> f64 {
    let (left, right) = (JsStr::from(left), JsStr::from(right));
    number(root_compare(
        realm,
        &left,
        &right,
        &options.unwrap_or(Value::Undefined),
    ))
}

#[test]
fn locale_comparison_collates_with_icu_in_the_requested_locale() {
    let mut realm = Realm::new();
    let compare = |realm: &mut Realm, left: &str, right: &str, locale: Value| {
        number(call(realm, &s(left), "localeCompare", &[s(right), locale]))
    };
    // Chinese sorts Han characters first, Latin letters after.
    assert_eq!(compare(&mut realm, "更改编码", "Bold", s("zh-CN")), -1.0);
    assert_eq!(compare(&mut realm, "更改编码", "Bold", s("en")), 1.0);
    assert_eq!(compare(&mut realm, "resume", "résumé", s("en")), -1.0);
    // The default collator is opened once and kept.
    let default = compare(&mut realm, "a", "b", Value::Undefined);
    assert_eq!(default, -1.0);
    assert!(realm.default_collator.is_some());
    assert_eq!(compare(&mut realm, "b", "a", Value::Undefined), 1.0);
    let numeric = options(&realm, &[("numeric", Value::Boolean(true))]);
    assert_eq!(
        number(call(
            &mut realm,
            &s("file10"),
            "localeCompare",
            &[s("file9"), Value::Undefined, numeric]
        )),
        1.0
    );
    let refused = call(&mut realm, &s("a"), "localeCompare", &[s("b"), s("!")]);
    assert!(message(&mut realm, refused).starts_with("RangeError"));
}

fn options(realm: &Realm, entries: &[(&str, Value)]) -> Value {
    let object = realm.object();
    for (key, value) in entries {
        object.define(Key::name(key), Property::data(value.clone()));
    }
    Value::Object(object)
}

#[test]
fn locale_comparison_orders_like_the_root_collation() {
    let mut realm = Realm::new();
    // Punctuation < digits < letters; letters ignore case at first.
    assert_eq!(compare(&mut realm, "-", "1", None), -1.0);
    assert_eq!(compare(&mut realm, "1", "a", None), -1.0);
    assert_eq!(compare(&mut realm, "a", "B", None), -1.0);
    assert_eq!(compare(&mut realm, "B", "a", None), 1.0);
    // Ties break by accents, then case with lowercase first.
    assert_eq!(compare(&mut realm, "resume", "résumé", None), -1.0);
    assert_eq!(compare(&mut realm, "a", "A", None), -1.0);
    assert_eq!(compare(&mut realm, "same", "same", None), 0.0);
    // Canonically equivalent spellings are equal.
    assert_eq!(compare(&mut realm, "é", "e\u{301}", None), 0.0);
    assert_eq!(compare(&mut realm, "file10", "file9", None), -1.0);
    let numeric = options(&realm, &[("numeric", Value::Boolean(true))]);
    assert_eq!(
        compare(&mut realm, "file10", "file9", Some(numeric.clone())),
        1.0
    );
    assert_eq!(
        compare(&mut realm, "file010", "file10", Some(numeric.clone())),
        0.0
    );
    assert_eq!(compare(&mut realm, "file2", "file10", Some(numeric)), -1.0);
    let base = options(&realm, &[("sensitivity", s("base"))]);
    assert_eq!(compare(&mut realm, "Résumé", "resume", Some(base)), 0.0);
    let accent = options(&realm, &[("sensitivity", s("accent"))]);
    assert_eq!(
        compare(&mut realm, "Résumé", "résumé", Some(accent.clone())),
        0.0
    );
    assert_eq!(compare(&mut realm, "resume", "résumé", Some(accent)), -1.0);
    let case = options(&realm, &[("sensitivity", s("case"))]);
    assert_eq!(
        compare(&mut realm, "resume", "résumé", Some(case.clone())),
        0.0
    );
    assert_eq!(compare(&mut realm, "a", "A", Some(case)), -1.0);
    let variant = options(
        &realm,
        &[("sensitivity", s("variant")), ("caseFirst", s("upper"))],
    );
    assert_eq!(compare(&mut realm, "a", "A", Some(variant)), 1.0);
    let lower = options(&realm, &[("caseFirst", s("false"))]);
    assert_eq!(compare(&mut realm, "a", "A", Some(lower)), -1.0);
    let ignore = options(&realm, &[("ignorePunctuation", Value::Boolean(true))]);
    assert_eq!(compare(&mut realm, "a-b", "ab", Some(ignore)), 0.0);
    assert_eq!(compare(&mut realm, "a-b", "ab", None), -1.0);
    // Full-width digits equal plain ones at base strength and sort after
    // them otherwise.
    let base = options(&realm, &[("sensitivity", s("base"))]);
    assert_eq!(compare(&mut realm, "\u{FF11}", "1", Some(base)), 0.0);
    assert_eq!(compare(&mut realm, "1", "\u{FF11}", None), -1.0);
    let numeric = options(&realm, &[("numeric", Value::Boolean(true))]);
    assert_eq!(
        compare(&mut realm, "item2.md", "item10.md", Some(numeric)),
        -1.0
    );
    // Compatibility decompositions may carry accents: U+01C4 is D, Z and a caron.
    let base = options(&realm, &[("sensitivity", s("base"))]);
    assert_eq!(compare(&mut realm, "\u{01C4}", "DZ", Some(base)), 0.0);
    assert_eq!(compare(&mut realm, "\u{01C4}", "DZ", None), 1.0);
    // A mark without a base character still compares.
    assert_eq!(compare(&mut realm, "\u{301}a", "a", None), 1.0);
    for (key, value) in [("sensitivity", "loud"), ("caseFirst", "middle")] {
        let invalid = options(&realm, &[(key, s(value))]);
        let refused = root_compare(&mut realm, &"a".into(), &"b".into(), &invalid);
        assert!(message(&mut realm, refused).starts_with("RangeError: Value out of range"));
    }
}

/// An object implementing a regular-expression protocol method that
/// returns its arguments joined.
fn protocol(realm: &Realm, symbol: Symbol) -> Value {
    let object = realm.object();
    let method = function(realm, |realm, _, args| {
        let mut text = String::new();
        for value in args {
            text.push_str(&ops::to_string(realm, value)?.to_rust_lossy());
            text.push('/');
        }
        Ok(Value::str(&text))
    });
    object.define(Key::Symbol(symbol), Property::data(method));
    Value::Object(object)
}

#[test]
fn replacing_and_splitting_strings_and_protocol_objects() {
    let mut realm = Realm::new();
    let text = s("a-b-c");
    assert_eq!(
        r!(realm, call(&mut realm, &text, "replace", &[s("-"), s("+")])),
        "a+b-c"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &text, "replace", &[s("x"), s("+")])),
        "a-b-c"
    );
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &text, "replaceAll", &[s("-"), s("+")])
        ),
        "a+b+c"
    );
    let template = s("[$$|$&|$`|$'|$1]");
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &s("xby"), "replace", &[s("b"), template])
        ),
        "x[$|b|x|y|$1]y"
    );
    let upper = function(&realm, |realm, _, args| {
        let matched = ops::to_string(realm, &args[0])?.to_rust_lossy();
        let position = ops::to_string(realm, &args[1])?.to_rust_lossy();
        Ok(Value::str(&format!("<{matched}{position}>")))
    });
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &text, "replaceAll", &[s("-"), upper.clone()])
        ),
        "a<-1>b<-3>c"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &text, "replace", &[s("b"), upper])),
        "a-<b2>-c"
    );
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &s("ab"), "replaceAll", &[s(""), s("_")])
        ),
        "_a_b_"
    );
    assert_eq!(
        r!(
            realm,
            call(&mut realm, &s("aaa"), "replaceAll", &[s("aa"), s("b")])
        ),
        "ba"
    );
    let thrower = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "replacer"))
    });
    for name in ["replace", "replaceAll"] {
        let thrown = call(&mut realm, &text, name, &[s("-"), thrower.clone()]);
        assert_eq!(
            message(&mut realm, thrown),
            "RangeError: replacer",
            "{name}"
        );
    }

    assert_eq!(
        p!(realm, call(&mut realm, &text, "split", &[s("-")])),
        "a|b|c"
    );
    assert_eq!(
        p!(realm, call(&mut realm, &text, "split", &[s("-"), n(2.0)])),
        "a|b"
    );
    assert_eq!(
        p!(realm, call(&mut realm, &text, "split", &[s("-"), n(0.0)])),
        ""
    );
    assert_eq!(p!(realm, call(&mut realm, &text, "split", &[])), "a-b-c");
    assert_eq!(
        p!(
            realm,
            call(&mut realm, &s("abc"), "split", &[s(""), n(2.0)])
        ),
        "a|b"
    );
    assert_eq!(p!(realm, call(&mut realm, &s(""), "split", &[s(",")])), "");
    assert_eq!(
        p!(realm, call(&mut realm, &s("a,"), "split", &[s(",")])),
        "a|"
    );

    // Objects with protocol methods receive the calls.
    let symbols = [
        ("replace", realm.symbols.replace.clone()),
        ("replaceAll", realm.symbols.replace.clone()),
        ("split", realm.symbols.split.clone()),
        ("match", realm.symbols.match_.clone()),
        ("search", realm.symbols.search.clone()),
    ];
    for (name, symbol) in symbols {
        let handler = protocol(&realm, symbol);
        let result = call(&mut realm, &s("text"), name, &[handler, s("arg")]);
        assert!(r!(realm, result).starts_with("text/"), "{name}");
    }
    // A protocol property that is not callable is rejected.
    let broken = realm.object();
    broken.define(
        Key::Symbol(realm.symbols.split.clone()),
        Property::data(n(1.0)),
    );
    let refused = call(&mut realm, &s("x"), "split", &[Value::Object(broken)]);
    assert!(message(&mut realm, refused).contains("is not a function"));
    for name in [
        "replace",
        "replaceAll",
        "split",
        "match",
        "matchAll",
        "search",
    ] {
        let refused = call(&mut realm, &Value::Null, name, &[s("a")]);
        assert!(
            message(&mut realm, refused).contains("called on null or undefined"),
            "{name}"
        );
    }
}

/// An object that regular-expression checks treat as a regular expression,
/// with the given `flags`.
fn regexp_like(realm: &Realm, flags: Value, all: Option<Symbol>) -> Value {
    let object = realm.object();
    object.define(
        Key::Symbol(realm.symbols.match_.clone()),
        Property::data(Value::Boolean(true)),
    );
    object.define(Key::name("flags"), Property::data(flags));
    if let Some(symbol) = all {
        let method = function(realm, |_, _, _| Ok(Value::str("delegated")));
        object.define(Key::Symbol(symbol), Property::data(method));
    }
    Value::Object(object)
}

#[test]
fn global_only_methods_check_regular_expression_flags() {
    let mut realm = Realm::new();
    let replace = Some(realm.symbols.replace.clone());
    let match_all = Some(realm.symbols.match_all.clone());
    for (name, symbol) in [("replaceAll", replace), ("matchAll", match_all)] {
        let missing = regexp_like(&realm, Value::Undefined, symbol.clone());
        let refused = call(&mut realm, &s("x"), name, &[missing, s("y")]);
        assert!(
            message(&mut realm, refused).contains("flags are null or undefined"),
            "{name}"
        );
        let local = regexp_like(&realm, s("i"), symbol.clone());
        let refused = call(&mut realm, &s("x"), name, &[local, s("y")]);
        assert!(
            message(&mut realm, refused).contains("global RegExp"),
            "{name}"
        );
        let global = regexp_like(&realm, s("gi"), symbol);
        let delegated = call(&mut realm, &s("x"), name, &[global, s("y")]);
        assert_eq!(r!(realm, delegated), "delegated", "{name}");
    }
    // String patterns become regular expressions from the intrinsic
    // constructor, even when the global binding is replaced.
    realm
        .global
        .define(Key::name("RegExp"), Property::hidden(n(1.0)));
    assert_eq!(
        r!(realm, call(&mut realm, &s("xpx"), "search", &[s("p")])),
        "1"
    );
    assert_eq!(r!(realm, call(&mut realm, &s("x"), "search", &[])), "0");
    assert_eq!(
        r!(realm, call(&mut realm, &s("x.y"), "match", &[s(".")])),
        "x"
    );
    let all = call(&mut realm, &s("ab"), "matchAll", &[s(".")]).unwrap();
    let next = ops::get(&mut realm, &all, &Key::name("next")).unwrap();
    let first = ops::call(&mut realm, &next, all, &[]).unwrap();
    let value = ops::get(&mut realm, &first, &Key::name("value")).unwrap();
    assert_eq!(r!(realm, Ok(value)), "a");
    let invalid = call(&mut realm, &s("x"), "match", &[s("(")]);
    assert!(message(&mut realm, invalid).starts_with("SyntaxError"));
}

#[test]
fn html_methods_wrap_strings_in_markup() {
    let mut realm = Realm::new();
    let anchor = call(&mut realm, &s("x"), "anchor", &[s("a\"b")]);
    assert_eq!(r!(realm, anchor), "<a name=\"a&quot;b\">x</a>");
    assert_eq!(
        r!(realm, call(&mut realm, &s("x"), "bold", &[])),
        "<b>x</b>"
    );
    assert_eq!(
        r!(realm, call(&mut realm, &s("x"), "fontsize", &[n(7.0)])),
        "<font size=\"7\">x</font>"
    );
    for name in [
        "big", "blink", "fixed", "italics", "small", "strike", "sub", "sup",
    ] {
        let result = call(&mut realm, &s("x"), name, &[]);
        assert!(r!(realm, result).starts_with('<'), "{name}");
    }
    let link = call(&mut realm, &s("x"), "link", &[s("u")]);
    assert_eq!(r!(realm, link), "<a href=\"u\">x</a>");
    let color = call(&mut realm, &s("x"), "fontcolor", &[s("red")]);
    assert_eq!(r!(realm, color), "<font color=\"red\">x</font>");
}

#[test]
fn abrupt_conversions_propagate() {
    let mut realm = Realm::new();
    let throwing = realm.object();
    let to_string = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "convert"))
    });
    throwing.define(Key::name("toString"), Property::data(to_string));
    let throwing = Value::Object(throwing);
    let names = [
        "at",
        "charAt",
        "concat",
        "endsWith",
        "includes",
        "indexOf",
        "lastIndexOf",
        "localeCompare",
        "normalize",
        "padEnd",
        "repeat",
        "replace",
        "replaceAll",
        "slice",
        "split",
        "startsWith",
        "substr",
        "substring",
        "toLowerCase",
        "trim",
        "isWellFormed",
        "toWellFormed",
        "codePointAt",
        "charCodeAt",
        "anchor",
    ];
    for name in names {
        let thrown = call(&mut realm, &throwing, name, &[]);
        assert_eq!(message(&mut realm, thrown), "RangeError: convert", "{name}");
    }
    // Methods that convert their first argument.
    for name in [
        "at",
        "charAt",
        "concat",
        "endsWith",
        "includes",
        "indexOf",
        "lastIndexOf",
        "localeCompare",
        "normalize",
        "padEnd",
        "repeat",
        "replace",
        "replaceAll",
        "slice",
        "split",
        "startsWith",
        "substr",
        "substring",
        "codePointAt",
        "charCodeAt",
        "anchor",
    ] {
        let thrown = call(
            &mut realm,
            &s("text"),
            name,
            &[throwing.clone(), throwing.clone()],
        );
        assert!(
            message(&mut realm, thrown).starts_with("RangeError"),
            "{name} argument"
        );
    }
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "option"))
    });
    let options = realm.object();
    options.define(
        Key::name("numeric"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let thrown = call(
        &mut realm,
        &s("a"),
        "localeCompare",
        &[s("b"), Value::Undefined, Value::Object(options)],
    );
    assert_eq!(message(&mut realm, thrown), "RangeError: option");
    let raw = statics(&mut realm, "raw", &[Value::Undefined]);
    assert!(message(&mut realm, raw).starts_with("TypeError"));
    let code = statics(&mut realm, "fromCharCode", std::slice::from_ref(&throwing));
    assert_eq!(message(&mut realm, code), "RangeError: convert");
    let code = statics(&mut realm, "fromCodePoint", &[throwing]);
    assert_eq!(message(&mut realm, code), "RangeError: convert");
}
