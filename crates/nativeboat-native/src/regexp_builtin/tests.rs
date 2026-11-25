//! Expected values are Node 22's results for the same operations.

use super::*;
use crate::symbol::Symbol;

#[test]
fn protected_intrinsics_preserve_constructor_identity_and_prototype_getters() {
    let mut realm = Realm::new();
    let public = global(&mut realm, "RegExp");
    let binding = crate::modules::load(&mut realm, "binding/util").unwrap();
    let getter = ops::get(&mut realm, &binding, &Key::name("getProtectedRegExp")).unwrap();
    let constructor = ops::call(&mut realm, &getter, binding, &[]).unwrap();
    assert!(!crate::same_value(&constructor, &public));
    let prototype = ops::get(&mut realm, &constructor, &Key::name("prototype")).unwrap();
    assert_eq!(
        prototype.as_object(),
        Some(&realm.protected_regexp_prototype)
    );
    assert!(crate::same_value(
        &ops::get(&mut realm, &prototype, &Key::name("source")).unwrap(),
        &s("(?:)")
    ));
    assert!(crate::same_value(
        &ops::get(&mut realm, &prototype, &Key::name("flags")).unwrap(),
        &s("")
    ));
    for (_, name) in ACCESSORS {
        assert!(matches!(
            ops::get(&mut realm, &prototype, &Key::name(name)).unwrap(),
            Value::Undefined
        ));
    }
    let regex = ops::construct(&mut realm, &constructor, &[s("a"), s("g")]).unwrap();
    let called = ops::call(
        &mut realm,
        &constructor,
        Value::Undefined,
        std::slice::from_ref(&regex),
    )
    .unwrap();
    assert!(crate::same_value(&called, &regex));
    let constructed =
        ops::construct(&mut realm, &constructor, std::slice::from_ref(&regex)).unwrap();
    assert!(!crate::same_value(&constructed, &regex));
    assert_eq!(
        constructed.as_object().unwrap().data().prototype.as_ref(),
        prototype.as_object()
    );
}

#[test]
fn protected_methods_survive_mutation_and_release_of_public_methods() {
    let mut realm = Realm::new();
    let constructor = Value::Object(realm.protected_regexp_constructor.clone().unwrap());
    let regex = ops::construct(&mut realm, &constructor, &[s("a"), s("g")]).unwrap();
    let public_prototype = realm.regexp_prototype.clone();
    let poison = realm.function("poison", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "public hook"))
    });
    public_prototype.define(
        Key::name("exec"),
        Property::data(Value::Object(poison.clone())),
    );
    public_prototype.define(
        Key::Symbol(realm.symbols.replace.clone()),
        Property::data(Value::Object(poison.clone())),
    );
    public_prototype.define(
        Key::Symbol(realm.symbols.split.clone()),
        Property::data(Value::Object(poison)),
    );
    public_prototype.define(Key::name("constructor"), Property::data(Value::Undefined));
    realm.set_global("RegExp", Value::Undefined);
    for (symbol, expected) in [
        (realm.symbols.replace.clone(), s("xbx")),
        (realm.symbols.split.clone(), Value::Undefined),
    ] {
        let method = ops::get(&mut realm, &regex, &Key::Symbol(symbol.clone())).unwrap();
        let args = if symbol == realm.symbols.replace {
            vec![s("aba"), s("x")]
        } else {
            vec![s("aba")]
        };
        let result = ops::call(&mut realm, &method, regex.clone(), &args).unwrap();
        if matches!(expected, Value::Undefined) {
            let join = ops::get(&mut realm, &result, &Key::name("join")).unwrap();
            let joined = ops::call(&mut realm, &join, result, &[s("|")]).unwrap();
            assert!(crate::same_value(&joined, &s("|b|")));
        } else {
            assert!(crate::same_value(&result, &expected));
        }
    }
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

/// `new RegExp(source, flags)`.
fn regexp(realm: &mut Realm, source: &str, flags: &str) -> Value {
    let constructor = global(realm, "RegExp");
    ops::construct(realm, &constructor, &[s(source), s(flags)]).unwrap()
}

fn invoke(realm: &mut Realm, target: &Value, key: Key, args: &[Value]) -> Completion {
    let method = ops::get(realm, target, &key)?;
    ops::call(realm, &method, target.clone(), args)
}

fn method(realm: &mut Realm, target: &Value, name: &str, args: &[Value]) -> Completion {
    invoke(realm, target, Key::name(name), args)
}

fn symbol(realm: &Realm, name: &str) -> Key {
    let symbols = &realm.symbols;
    Key::Symbol(match name {
        "match" => symbols.match_.clone(),
        "matchAll" => symbols.match_all.clone(),
        "replace" => symbols.replace.clone(),
        "search" => symbols.search.clone(),
        "split" => symbols.split.clone(),
        _ => symbols.species.clone(),
    })
}

/// A value as Node's `util.inspect` prints it.
fn show(realm: &mut Realm, value: Completion) -> String {
    let value = value.unwrap();
    crate::inspect::inspect(realm, &value, 4).unwrap()
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

/// Calls a String.prototype method on text.
fn string(realm: &mut Realm, text: &str, name: &str, args: &[Value]) -> Completion {
    method(realm, &s(text), name, args)
}

macro_rules! check {
    ($realm:ident, $value:expr, $expected:expr) => {{
        let value = $value;
        assert_eq!(show(&mut $realm, value), $expected);
    }};
}

macro_rules! fails {
    ($realm:ident, $value:expr, $expected:expr) => {{
        let value = $value;
        assert_eq!(message(&mut $realm, value), $expected);
    }};
}

fn function(
    realm: &Realm,
    constructor: bool,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Value {
    Value::Object(realm.function("f", 1, constructor, code))
}

#[test]
fn construction_validates_flags_and_syntax() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "RegExp");
    let create = |realm: &mut Realm, source: &str, flags: &str| {
        let constructor = global(realm, "RegExp");
        ops::construct(realm, &constructor, &[s(source), s(flags)])
    };
    for flags in ["gg", "uv", "x", "g\u{100}"] {
        fails!(
            realm,
            create(&mut realm, "a", flags),
            format!("SyntaxError: Invalid flags supplied to RegExp constructor '{flags}'")
        );
    }
    let result = create(&mut realm, "(", "");
    let unterminated = message(&mut realm, result);
    assert!(
        unterminated.starts_with("SyntaxError: Invalid regular expression: /(/: "),
        "{unterminated}"
    );
    fails!(
        realm,
        create(&mut realm, "(?<a>x)(?<a>y)", ""),
        "SyntaxError: Invalid regular expression: /(?<a>x)(?<a>y)/: Duplicate capture group name"
    );
    check!(
        realm,
        ops::get(&mut realm, &constructor, &Key::name("length")),
        "2"
    );
    let empty = ops::construct(&mut realm, &constructor, &[]).unwrap();
    check!(
        realm,
        ops::get(&mut realm, &empty, &Key::name("source")),
        "'(?:)'"
    );
    let descriptor = empty
        .as_object()
        .unwrap()
        .own_property(&last_index_key())
        .unwrap();
    assert!(!descriptor.enumerable && !descriptor.configurable);
    assert!(matches!(
        descriptor.slot,
        Slot::Data {
            writable: true,
            value: Value::Number(0.0)
        }
    ));
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let prototype = Value::Object(realm.regexp_prototype.clone());
    assert!(ops::get(&mut realm, &prototype, &tag).unwrap().is_nullish());
}

#[test]
fn calls_return_regexps_and_construction_copies_them() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "RegExp");
    let original = regexp(&mut realm, "x", "g");
    let same = ops::call(
        &mut realm,
        &constructor,
        Value::Undefined,
        std::slice::from_ref(&original),
    )
    .unwrap();
    assert_eq!(same.as_object(), original.as_object());
    let flagged = ops::call(
        &mut realm,
        &constructor,
        Value::Undefined,
        &[original.clone(), s("i")],
    );
    check!(realm, flagged, "/x/i");
    let copied = ops::construct(&mut realm, &constructor, std::slice::from_ref(&original)).unwrap();
    assert_ne!(copied.as_object(), original.as_object());
    check!(realm, Ok(copied), "/x/g");
    // A regular expression whose constructor differs is copied by calls.
    let other = regexp(&mut realm, "y", "");
    ops::set(&mut realm, &other, &Key::name("constructor"), n(1.0)).unwrap();
    let call = ops::call(
        &mut realm,
        &constructor,
        Value::Undefined,
        std::slice::from_ref(&other),
    )
    .unwrap();
    assert_ne!(call.as_object(), other.as_object());
    // Objects marked by Symbol.match supply source and flags properties.
    let like = Value::Object(realm.object());
    let matcher = Key::Symbol(realm.symbols.match_.clone());
    for (key, value) in [
        (matcher, Value::Boolean(true)),
        (Key::name("source"), s("q")),
        (Key::name("flags"), s("m")),
    ] {
        ops::set(&mut realm, &like, &key, value).unwrap();
    }
    let built = ops::call(
        &mut realm,
        &constructor,
        Value::Undefined,
        std::slice::from_ref(&like),
    );
    check!(realm, built, "/q/m");
    let built = ops::construct(&mut realm, &constructor, &[like, s("s")]);
    check!(realm, built, "/q/s");
    // Construction uses the new target's prototype, or the intrinsic one.
    let target = function(&realm, true, |_, _, _| Ok(Value::Undefined));
    let prototype = realm.object();
    prototype.set_prototype(Some(realm.regexp_prototype.clone()));
    ops::set(
        &mut realm,
        &target,
        &Key::name("prototype"),
        Value::Object(prototype.clone()),
    )
    .unwrap();
    let derived = ops::construct_target(&mut realm, &constructor, &[s("a")], &target).unwrap();
    assert_eq!(
        derived.as_object().unwrap().data().prototype,
        Some(prototype)
    );
    ops::set(&mut realm, &target, &Key::name("prototype"), n(1.0)).unwrap();
    let plain = ops::construct_target(&mut realm, &constructor, &[s("a")], &target).unwrap();
    assert_eq!(
        plain.as_object().unwrap().data().prototype,
        Some(realm.regexp_prototype.clone())
    );
    let species = Key::Symbol(realm.symbols.species.clone());
    let own = ops::get(&mut realm, &constructor, &species).unwrap();
    assert_eq!(own.as_object(), constructor.as_object());
}

#[test]
fn exec_builds_match_arrays_and_maintains_last_index() {
    let mut realm = Realm::new();
    let pattern = regexp(&mut realm, "(a)|(b)", "");
    check!(
        realm,
        method(&mut realm, &pattern, "exec", &[s("b")]),
        "[ 'b', undefined, 'b', index: 0, input: 'b', groups: undefined ]"
    );
    let indices = regexp(&mut realm, "a(?<n>b)?", "d");
    check!(
        realm,
        method(&mut realm, &indices, "exec", &[s("xab")]),
        "[\n  'ab',\n  'b',\n  index: 1,\n  input: 'xab',\n  groups: [Object: null prototype] { n: 'b' },\n  indices: [\n    [ 1, 3 ],\n    [ 2, 3 ],\n    groups: [Object: null prototype] { n: [ 2, 3 ] }\n  ]\n]"
    );
    let plain = regexp(&mut realm, "(x)?", "d");
    check!(
        realm,
        method(&mut realm, &plain, "exec", &[s("")]),
        "[\n  '',\n  undefined,\n  index: 0,\n  input: '',\n  groups: undefined,\n  indices: [ [ 0, 0 ], undefined, groups: undefined ]\n]"
    );
    let year = regexp(&mut realm, "(?<year>\\d{4})", "");
    let found = method(&mut realm, &year, "exec", &[s("in 2026")]).unwrap();
    check!(
        realm,
        ops::get(&mut realm, &found, &Key::name("groups")),
        "[Object: null prototype] { year: '2026' }"
    );

    // Global searches start at and update lastIndex.
    let global_a = regexp(&mut realm, "a", "g");
    ops::set(&mut realm, &global_a, &last_index_key(), n(5.0)).unwrap();
    check!(
        realm,
        method(&mut realm, &global_a, "test", &[s("aaaaaaa")]),
        "true"
    );
    check!(
        realm,
        ops::get(&mut realm, &global_a, &last_index_key()),
        "6"
    );
    ops::set(&mut realm, &global_a, &last_index_key(), n(9.0)).unwrap();
    check!(
        realm,
        method(&mut realm, &global_a, "exec", &[s("aaa")]),
        "null"
    );
    check!(
        realm,
        ops::get(&mut realm, &global_a, &last_index_key()),
        "0"
    );
    // Other searches ignore it.
    let once = regexp(&mut realm, "a", "");
    ops::set(&mut realm, &once, &last_index_key(), n(9.0)).unwrap();
    check!(realm, method(&mut realm, &once, "test", &[s("a")]), "true");
    check!(realm, method(&mut realm, &once, "test", &[s("b")]), "false");
    // Sticky searches match only at lastIndex.
    let sticky = regexp(&mut realm, "a", "y");
    check!(
        realm,
        method(&mut realm, &sticky, "exec", &[s("ba")]),
        "null"
    );
    ops::set(&mut realm, &sticky, &last_index_key(), n(1.0)).unwrap();
    check!(
        realm,
        method(&mut realm, &sticky, "exec", &[s("ba")]),
        "[ 'a', index: 1, input: 'ba', groups: undefined ]"
    );
    check!(realm, ops::get(&mut realm, &sticky, &last_index_key()), "2");
    ops::set(&mut realm, &sticky, &last_index_key(), n(0.0)).unwrap();
    check!(
        realm,
        string(&mut realm, "baa", "replace", &[sticky.clone(), s("X")]),
        "'baa'"
    );
    check!(realm, ops::get(&mut realm, &sticky, &last_index_key()), "0");
    // Unicode searches starting inside a pair start at the pair, as in Node.
    let unicode = regexp(&mut realm, ".", "gu");
    ops::set(&mut realm, &unicode, &last_index_key(), n(1.0)).unwrap();
    check!(
        realm,
        method(&mut realm, &unicode, "exec", &[s("😀")]),
        "[ '😀', index: 0, input: '😀', groups: undefined ]"
    );

    let object = Value::Object(realm.object());
    let exec = method(&mut realm, &pattern, "exec", &[]).map(|_| ());
    assert!(exec.is_ok());
    let bad = {
        let exec = ops::get(&mut realm, &pattern, &Key::name("exec")).unwrap();
        ops::call(&mut realm, &exec, object, &[s("a")])
    };
    fails!(
        realm,
        bad,
        "TypeError: Method RegExp.prototype.exec called on incompatible receiver"
    );
}

#[test]
fn test_and_protocols_call_user_exec_methods() {
    let mut realm = Realm::new();
    let custom = Value::Object(realm.object());
    let one = function(&realm, false, |_, _, _| Ok(n(1.0)));
    ops::set(&mut realm, &custom, &Key::name("exec"), one).unwrap();
    let test = {
        let pattern = regexp(&mut realm, "a", "");
        ops::get(&mut realm, &pattern, &Key::name("test")).unwrap()
    };
    let result = ops::call(&mut realm, &test, custom.clone(), &[s("a")]);
    fails!(
        realm,
        result,
        "TypeError: RegExp exec method returned something other than an Object or null"
    );
    let found = function(&realm, false, |realm, _, _| {
        Ok(Value::Object(realm.object()))
    });
    ops::set(&mut realm, &custom, &Key::name("exec"), found).unwrap();
    check!(
        realm,
        ops::call(&mut realm, &test, custom.clone(), &[s("a")]),
        "true"
    );
    // Without a callable exec, only RegExp objects can be searched.
    ops::set(&mut realm, &custom, &Key::name("exec"), n(1.0)).unwrap();
    let result = ops::call(&mut realm, &test, custom, &[s("a")]);
    fails!(
        realm,
        result,
        "TypeError: RegExp exec method called on an incompatible receiver"
    );
    let result = ops::call(&mut realm, &test, n(1.0), &[s("a")]);
    fails!(
        realm,
        result,
        "TypeError: Method RegExp.prototype.test called on incompatible receiver 1"
    );
    let pattern = regexp(&mut realm, "a", "");
    for name in ["match", "matchAll", "replace", "search", "split"] {
        let key = symbol(&realm, name);
        let protocol = ops::get(&mut realm, &pattern, &key).unwrap();
        let result = ops::call(&mut realm, &protocol, n(1.0), &[s("a")]);
        fails!(
            realm,
            result,
            format!(
                "TypeError: Method RegExp.prototype.@@{name} called on incompatible receiver 1"
            )
        );
    }
}

#[test]
fn to_string_flags_and_source_follow_the_accessors() {
    let mut realm = Realm::new();
    let slash = regexp(&mut realm, "a/b", "g");
    check!(
        realm,
        method(&mut realm, &slash, "toString", &[]),
        "'/a\\\\/b/g'"
    );
    let generic = Value::Object(realm.object());
    ops::set(&mut realm, &generic, &Key::name("source"), s("x")).unwrap();
    ops::set(&mut realm, &generic, &Key::name("flags"), s("q")).unwrap();
    let to_string = ops::get(&mut realm, &slash, &Key::name("toString")).unwrap();
    check!(
        realm,
        ops::call(&mut realm, &to_string, generic.clone(), &[]),
        "'/x/q'"
    );
    let result = ops::call(&mut realm, &to_string, n(1.0), &[]);
    fails!(
        realm,
        result,
        "TypeError: Method RegExp.prototype.toString called on incompatible receiver 1"
    );

    let all = regexp(&mut realm, "a", "ydsmigu");
    check!(
        realm,
        ops::get(&mut realm, &all, &Key::name("flags")),
        "'dgimsuy'"
    );
    let sets = regexp(&mut realm, "a", "v");
    check!(
        realm,
        ops::get(&mut realm, &sets, &Key::name("flags")),
        "'v'"
    );
    check!(
        realm,
        ops::get(&mut realm, &sets, &Key::name("unicodeSets")),
        "true"
    );
    check!(
        realm,
        ops::get(&mut realm, &sets, &Key::name("dotAll")),
        "false"
    );
    let prototype = Value::Object(realm.regexp_prototype.clone());
    check!(
        realm,
        ops::get(&mut realm, &prototype, &Key::name("flags")),
        "''"
    );
    check!(
        realm,
        ops::get(&mut realm, &prototype, &Key::name("global")),
        "undefined"
    );
    check!(
        realm,
        ops::get(&mut realm, &prototype, &Key::name("dotAll")),
        "undefined"
    );
    check!(
        realm,
        ops::get(&mut realm, &prototype, &Key::name("source")),
        "'(?:)'"
    );
    check!(
        realm,
        method(&mut realm, &prototype, "toString", &[]),
        "'/(?:)/'"
    );
    for name in ["global", "dotAll", "source"] {
        let descriptor = realm
            .regexp_prototype
            .own_property(&Key::name(name))
            .unwrap();
        let Slot::Accessor {
            get: Some(getter), ..
        } = descriptor.slot
        else {
            unreachable!()
        };
        let result = ops::call(&mut realm, &Value::Object(getter), generic.clone(), &[]);
        fails!(
            realm,
            result,
            format!("TypeError: RegExp.prototype.{name} getter called on non-RegExp object")
        );
    }
    let descriptor = realm
        .regexp_prototype
        .own_property(&Key::name("flags"))
        .unwrap();
    let Slot::Accessor {
        get: Some(getter), ..
    } = descriptor.slot
    else {
        unreachable!()
    };
    let result = ops::call(&mut realm, &Value::Object(getter), n(1.0), &[]);
    fails!(
        realm,
        result,
        "TypeError: RegExp.prototype.flags getter called on non-object"
    );

    let sources = [
        "/",
        "[/]",
        "\\/",
        "\n",
        "\\\n",
        "\r\u{2028}\u{2029}",
        "[\\]/]",
        "a\\\\/",
        "[\n]",
        "a\\",
    ];
    let expected = [
        "\\/",
        "[/]",
        "\\/",
        "\\n",
        "\\n",
        "\\r\\u2028\\u2029",
        "[\\]/]",
        "a\\\\\\/",
        "[\\n]",
        "a\\",
    ];
    for (source, expected) in sources.into_iter().zip(expected) {
        let units: Vec<u16> = source.encode_utf16().collect();
        assert_eq!(
            escape_source(&units).to_rust_lossy(),
            expected,
            "{source:?}"
        );
    }
}

#[test]
fn match_and_match_all_iterate_matches() {
    let mut realm = Realm::new();
    let lazy = regexp(&mut realm, "a*?", "g");
    check!(
        realm,
        string(&mut realm, "aaa", "match", &[lazy]),
        "[ '', '', '', '' ]"
    );
    let missing = regexp(&mut realm, "x", "g");
    check!(
        realm,
        string(&mut realm, "aaa", "match", &[missing]),
        "null"
    );
    let first = regexp(&mut realm, "a", "");
    check!(
        realm,
        string(&mut realm, "ba", "match", &[first]),
        "[ 'a', index: 1, input: 'ba', groups: undefined ]"
    );
    let pairs = regexp(&mut realm, "(?:)", "gu");
    check!(
        realm,
        string(&mut realm, "😀😀", "match", &[pairs]),
        "[ '', '', '' ]"
    );
    let units = regexp(&mut realm, "(?:)", "g");
    check!(
        realm,
        string(&mut realm, "😀", "match", &[units]),
        "[ '', '', '' ]"
    );
    check!(
        realm,
        string(&mut realm, "ab", "match", &[s("b")]),
        "[ 'b', index: 1, input: 'ab', groups: undefined ]"
    );

    let digits = regexp(&mut realm, "\\d", "g");
    let iterator = string(
        &mut realm,
        "a1b2",
        "matchAll",
        std::slice::from_ref(&digits),
    )
    .unwrap();
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    check!(
        realm,
        ops::get(&mut realm, &iterator, &tag),
        "'RegExp String Iterator'"
    );
    check!(
        realm,
        method(&mut realm, &iterator, "next", &[]),
        "{\n  value: [ '1', index: 1, input: 'a1b2', groups: undefined ],\n  done: false\n}"
    );
    check!(
        realm,
        method(&mut realm, &iterator, "next", &[]),
        "{\n  value: [ '2', index: 3, input: 'a1b2', groups: undefined ],\n  done: false\n}"
    );
    check!(
        realm,
        method(&mut realm, &iterator, "next", &[]),
        "{ value: undefined, done: true }"
    );
    check!(
        realm,
        method(&mut realm, &iterator, "next", &[]),
        "{ value: undefined, done: true }"
    );
    // Empty global matches advance; the source's lastIndex is untouched.
    let empty = regexp(&mut realm, "", "g");
    let iterator = string(&mut realm, "a", "matchAll", std::slice::from_ref(&empty)).unwrap();
    for done in ["false", "false", "true"] {
        let next = method(&mut realm, &iterator, "next", &[]).unwrap();
        check!(realm, ops::get(&mut realm, &next, &Key::name("done")), done);
    }
    check!(realm, ops::get(&mut realm, &empty, &last_index_key()), "0");
    // Non-global iterators yield once.
    let once = regexp(&mut realm, "a", "");
    let key = symbol(&realm, "matchAll");
    let iterator = invoke(&mut realm, &once, key, &[s("aa")]).unwrap();
    for done in ["false", "true"] {
        let next = method(&mut realm, &iterator, "next", &[]).unwrap();
        check!(realm, ops::get(&mut realm, &next, &Key::name("done")), done);
    }
    let next = ops::get(&mut realm, &iterator, &Key::name("next")).unwrap();
    let plain = Value::Object(realm.object());
    for receiver in [plain, n(1.0)] {
        let result = ops::call(&mut realm, &next, receiver, &[]);
        fails!(
            realm,
            result,
            "TypeError: Method %RegExpStringIterator%.prototype.next called on incompatible receiver"
        );
    }
    fails!(
        realm,
        string(&mut realm, "a", "matchAll", &[once]),
        "TypeError: matchAll must be called with a global RegExp"
    );
}

#[test]
fn species_constructors_are_validated() {
    let mut realm = Realm::new();
    let pattern = regexp(&mut realm, "a", "g");
    let key = symbol(&realm, "matchAll");
    ops::set(&mut realm, &pattern, &Key::name("constructor"), n(1.0)).unwrap();
    fails!(
        realm,
        invoke(&mut realm, &pattern, key.clone(), &[s("a")]),
        "TypeError: object.constructor is not an object"
    );
    let constructor = Value::Object(realm.object());
    ops::set(
        &mut realm,
        &pattern,
        &Key::name("constructor"),
        constructor.clone(),
    )
    .unwrap();
    let species = symbol(&realm, "species");
    check!(
        realm,
        invoke(&mut realm, &pattern, key.clone(), &[s("a")]).map(|_| Value::Null),
        "null"
    );
    // An undefined constructor falls back to %RegExp%.
    let unset = regexp(&mut realm, "a", "g");
    ops::set(
        &mut realm,
        &unset,
        &Key::name("constructor"),
        Value::Undefined,
    )
    .unwrap();
    let iterator = invoke(&mut realm, &unset, key.clone(), &[s("a")]).unwrap();
    check!(
        realm,
        method(&mut realm, &iterator, "next", &[]).map(|_| Value::Null),
        "null"
    );
    ops::set(&mut realm, &constructor, &species, n(1.0)).unwrap();
    fails!(
        realm,
        invoke(&mut realm, &pattern, key.clone(), &[s("a")]),
        "TypeError: object.constructor[Symbol.species] is not a constructor"
    );
    let regexp_constructor = global(&mut realm, "RegExp");
    ops::set(&mut realm, &constructor, &species, regexp_constructor).unwrap();
    let iterator = invoke(&mut realm, &pattern, key, &[s("a")]).unwrap();
    check!(
        realm,
        method(&mut realm, &iterator, "next", &[]),
        "{\n  value: [ 'a', index: 0, input: 'a', groups: undefined ],\n  done: false\n}"
    );
}

#[test]
fn replace_substitutes_references_and_calls_replacers() {
    let mut realm = Realm::new();
    let named = regexp(&mut realm, "(?<x>b)", "");
    let template = s("[$<x>|$1|$&|$`|$'|$$|$2|$<y>|$<x|$0|$00|$|$z]");
    check!(
        realm,
        string(&mut realm, "abc", "replace", &[named.clone(), template]),
        "'a[b|b|b|a|c|$|$2||$<x|$0|$00|$|$z]c'"
    );
    let one = regexp(&mut realm, "(b)", "");
    check!(
        realm,
        string(&mut realm, "abc", "replace", &[one.clone(), s("$10|$01")]),
        "'ab0|bc'"
    );
    check!(
        realm,
        string(&mut realm, "abc", "replace", &[one, s("$<x>")]),
        "'a$<x>c'"
    );
    let twelve = regexp(&mut realm, "(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)(k)(l)", "");
    check!(
        realm,
        string(
            &mut realm,
            "abcdefghijkl",
            "replace",
            &[twelve, s("$12$11")]
        ),
        "'lk'"
    );
    let optional = regexp(&mut realm, "(x)?b", "");
    check!(
        realm,
        string(&mut realm, "ab", "replace", &[optional, s("[$1]")]),
        "'a[]'"
    );
    let numbered = regexp(&mut realm, "b", "");
    check!(
        realm,
        string(&mut realm, "abc", "replace", &[numbered, n(7.0)]),
        "'a7c'"
    );
    let empty = regexp(&mut realm, "a*", "g");
    check!(
        realm,
        string(&mut realm, "aaa", "replace", &[empty, s("-")]),
        "'--'"
    );
    let dash = regexp(&mut realm, "(-)", "g");
    let replacer = function(&realm, false, |realm, _, args| {
        let parts: Vec<String> = args
            .iter()
            .map(|value| crate::inspect::inspect(realm, value, 0).unwrap())
            .collect();
        Ok(Value::str(&parts.join(",")))
    });
    check!(
        realm,
        string(&mut realm, "a-b", "replace", &[dash, replacer.clone()]),
        "\"a'-','-',1,'a-b'b\""
    );
    let group = regexp(&mut realm, "(?<x>a)(z)?", "");
    check!(
        realm,
        string(&mut realm, "ab", "replace", &[group, replacer]),
        "\"'a','a',undefined,0,'ab',[Object: null prototype] { x: 'a' }b\""
    );

    // Results from a user exec: overlapping and out-of-range positions,
    // and named captures converted with ToObject.
    let custom = regexp(&mut realm, "a", "g");
    let calls = Rc::new(core::cell::Cell::new(0));
    let counter = calls.clone();
    let exec = function(&realm, false, move |realm, _, _| {
        counter.set(counter.get() + 1);
        let (index, groups) = match counter.get() {
            1 => (n(2.0), s("g")),
            2 => (n(1.0), Value::Undefined),
            3 => (n(99.0), Value::Undefined),
            _ => return Ok(Value::Null),
        };
        let result = realm.array(vec![s("x")]);
        result.define(Key::name("index"), Property::data(index));
        result.define(Key::name("groups"), Property::data(groups));
        Ok(Value::Object(result))
    });
    ops::set(&mut realm, &custom, &Key::name("exec"), exec).unwrap();
    check!(
        realm,
        string(&mut realm, "abcd", "replace", &[custom, s("<$<length>>")]),
        "'ab<1>d<$<length>>'"
    );
    assert_eq!(calls.get(), 4);
}

#[test]
fn search_restores_last_index() {
    let mut realm = Realm::new();
    let pattern = regexp(&mut realm, "c", "g");
    ops::set(&mut realm, &pattern, &last_index_key(), n(3.0)).unwrap();
    check!(
        realm,
        string(&mut realm, "abc", "search", std::slice::from_ref(&pattern)),
        "2"
    );
    check!(
        realm,
        ops::get(&mut realm, &pattern, &last_index_key()),
        "3"
    );
    ops::set(&mut realm, &pattern, &last_index_key(), n(0.0)).unwrap();
    check!(
        realm,
        string(&mut realm, "ab", "search", std::slice::from_ref(&pattern)),
        "-1"
    );
    check!(
        realm,
        ops::get(&mut realm, &pattern, &last_index_key()),
        "0"
    );
    check!(realm, string(&mut realm, "abc", "search", &[s("b")]), "1");
}

#[test]
fn split_follows_the_sticky_splitter_protocol() {
    let mut realm = Realm::new();
    let cases: [(&str, &str, &str, Option<f64>, &str); 10] = [
        ("a,b,,c", ",", "", None, "[ 'a', 'b', '', 'c' ]"),
        ("abc", "(b)", "", None, "[ 'a', 'b', 'c' ]"),
        ("abc", "(b)", "", Some(2.0), "[ 'a', 'b' ]"),
        ("", "x", "", None, "[ '' ]"),
        ("", "(?:)", "", None, "[]"),
        ("ab", "(?:)", "", None, "[ 'a', 'b' ]"),
        ("aXbX", "x", "i", Some(1.0), "[ 'a' ]"),
        ("ab", "x", "y", Some(0.0), "[]"),
        ("😀😀", "(?:)", "u", None, "[ '😀', '😀' ]"),
        ("😀", "(?:)", "", None, "[ '\\ud83d', '\\ude00' ]"),
    ];
    for (text, source, flags, limit, expected) in cases {
        let pattern = regexp(&mut realm, source, flags);
        let mut args = vec![pattern];
        args.extend(limit.map(n));
        check!(realm, string(&mut realm, text, "split", &args), expected);
    }
    // A splitter without the built-in exec is tried at each position.
    let pattern = regexp(&mut realm, ",", "");
    let positions = Rc::new(RefCell::new(Vec::new()));
    let seen = positions.clone();
    let species = function(&realm, true, move |realm, _, args| {
        let constructor = global(realm, "RegExp");
        let splitter = ops::construct(realm, &constructor, args)?;
        let seen = seen.clone();
        let exec = realm.function("exec", 1, false, move |realm, this, args| {
            let index = ops::get(realm, &this, &last_index_key())?;
            let index = ops::to_number(realm, &index)?;
            seen.borrow_mut().push(index);
            let data = data_of(&this).unwrap();
            let string = ops::to_string(realm, &args[0])?;
            builtin_exec(realm, &this, &data, &string)
        });
        ops::set(realm, &splitter, &Key::name("exec"), Value::Object(exec))?;
        Ok(splitter)
    });
    let constructor = Value::Object(realm.object());
    let key = symbol(&realm, "species");
    ops::set(&mut realm, &constructor, &key, species).unwrap();
    ops::set(&mut realm, &pattern, &Key::name("constructor"), constructor).unwrap();
    check!(
        realm,
        string(&mut realm, "a,b", "split", std::slice::from_ref(&pattern)),
        "[ 'a', 'b' ]"
    );
    assert_eq!(*positions.borrow(), [0.0, 1.0, 2.0]);
    check!(realm, string(&mut realm, "", "split", &[pattern]), "[ '' ]");
    // A user exec whose match leaves lastIndex at the previous split.
    let stuck = regexp(&mut realm, "a", "");
    let species = function(&realm, true, |realm, _, _| {
        let splitter = Value::Object(realm.object());
        let exec = realm.function("exec", 1, false, |realm, _, _| {
            Ok(Value::Object(realm.array(vec![s("")])))
        });
        ops::set(realm, &splitter, &Key::name("exec"), Value::Object(exec))?;
        Ok(splitter)
    });
    let constructor = Value::Object(realm.object());
    ops::set(&mut realm, &constructor, &key, species).unwrap();
    ops::set(&mut realm, &stuck, &Key::name("constructor"), constructor).unwrap();
    check!(
        realm,
        string(&mut realm, "ab", "split", &[stuck]),
        "[ 'a', 'b' ]"
    );
}

#[test]
fn literals_share_compiled_patterns_and_the_cache_is_bounded() {
    let mut realm = Realm::new();
    let first = literal(&mut realm, &[0x61], "g").unwrap();
    let second = literal(&mut realm, &[0x61], "g").unwrap();
    assert_ne!(first.as_object(), second.as_object());
    let (a, b) = (data_of(&first).unwrap(), data_of(&second).unwrap());
    assert!(Rc::ptr_eq(&a.pattern, &b.pattern));
    check!(realm, Ok(first), "/a/g");
    for index in 0..CACHE_LIMIT {
        let source: Vec<u16> = format!("x{index}").encode_utf16().collect();
        literal(&mut realm, &source, "").unwrap();
    }
    assert_eq!(realm.patterns.count, 1);
    let third = literal(&mut realm, &[0x61], "g").unwrap();
    assert!(!Rc::ptr_eq(&a.pattern, &data_of(&third).unwrap().pattern));
    let invalid = literal(&mut realm, &[0x28], "");
    assert!(message(&mut realm, invalid).starts_with("SyntaxError"));
}

#[test]
fn string_methods_recognize_regexps() {
    let mut realm = Realm::new();
    let pattern = regexp(&mut realm, "a", "");
    assert!(is_regexp(&mut realm, &pattern).unwrap());
    let key = Key::Symbol(realm.symbols.match_.clone());
    ops::set(&mut realm, &pattern, &key, Value::Boolean(false)).unwrap();
    assert!(!is_regexp(&mut realm, &pattern).unwrap());
    check!(
        realm,
        string(&mut realm, "/a/", "startsWith", &[pattern]),
        "true"
    );
    let marked = Value::Object(realm.object());
    ops::set(&mut realm, &marked, &key, n(1.0)).unwrap();
    assert!(is_regexp(&mut realm, &marked).unwrap());
    let plain = Value::Object(realm.object());
    assert!(!is_regexp(&mut realm, &plain).unwrap());
    assert!(!is_regexp(&mut realm, &n(1.0)).unwrap());
    let once = regexp(&mut realm, "a", "");
    fails!(
        realm,
        string(&mut realm, "a", "replaceAll", &[once, s("b")]),
        "TypeError: replaceAll must be called with a global RegExp"
    );
    let every = regexp(&mut realm, "a", "g");
    check!(
        realm,
        string(&mut realm, "aba", "replaceAll", &[every, s("$&$&")]),
        "'aabaa'"
    );
    check!(
        realm,
        string(&mut realm, "aXa", "replace", &[s("X"), s("[$`$'$1$<x>]")]),
        "'a[aa$1$<x>]a'"
    );
    let _ = Symbol::new(None);
}

#[test]
fn regexps_inspect_as_literals() {
    let mut realm = Realm::new();
    let pattern = regexp(&mut realm, "a/b", "gi");
    check!(realm, Ok(pattern.clone()), "/a\\/b/gi");
    ops::set(&mut realm, &pattern, &Key::name("x"), n(1.0)).unwrap();
    check!(realm, Ok(pattern), "/a\\/b/gi { x: 1 }");
    let derived = regexp(&mut realm, "c", "");
    let prototype = realm.object();
    prototype.set_prototype(Some(realm.regexp_prototype.clone()));
    let constructor = function(&realm, true, |_, _, _| Ok(Value::Undefined));
    ops::set(&mut realm, &constructor, &Key::name("name"), s("R")).ok();
    constructor
        .as_object()
        .unwrap()
        .define(Key::name("name"), Property::hidden(s("R")));
    constructor.as_object().unwrap().define(
        Key::name("prototype"),
        Property::hidden(Value::Object(prototype.clone())),
    );
    prototype.define(Key::name("constructor"), Property::hidden(constructor));
    derived.as_object().unwrap().set_prototype(Some(prototype));
    check!(realm, Ok(derived.clone()), "R /c/");
    derived.as_object().unwrap().set_prototype(None);
    check!(realm, Ok(derived), "[RegExp: null prototype] /c/");
}

#[test]
fn abrupt_completions_and_fallbacks_propagate() {
    let mut realm = Realm::new();
    // Unicode empty matches advance one unit outside surrogate pairs.
    let empty = regexp(&mut realm, "(?:)", "gu");
    check!(
        realm,
        string(&mut realm, "ab", "match", &[empty]),
        "[ '', '', '' ]"
    );
    // A user exec that throws.
    let throwing = regexp(&mut realm, "a", "");
    let thrower = function(&realm, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "exec failed"))
    });
    ops::set(&mut realm, &throwing, &Key::name("exec"), thrower.clone()).unwrap();
    fails!(
        realm,
        method(&mut realm, &throwing, "test", &[s("a")]),
        "RangeError: exec failed"
    );
    // A RegExp whose exec is not callable searches with the built-in one.
    let plain = regexp(&mut realm, "a", "");
    ops::set(&mut realm, &plain, &Key::name("exec"), n(1.0)).unwrap();
    check!(
        realm,
        method(&mut realm, &plain, "test", &[s("ba")]),
        "true"
    );
    // Species lookups and constructions that throw.
    let getter = |realm: &mut Realm, target: &Value, key: Key| {
        let thrower = realm.function("get", 0, false, |realm, _, _| {
            Err(realm.throw(ErrorKind::Range, "getter failed"))
        });
        target.as_object().unwrap().define(
            key,
            Property {
                slot: Slot::Accessor {
                    get: Some(thrower),
                    set: None,
                },
                enumerable: false,
                configurable: true,
            },
        );
    };
    let pattern = regexp(&mut realm, "a", "g");
    getter(&mut realm, &pattern, Key::name("constructor"));
    let key = symbol(&realm, "matchAll");
    fails!(
        realm,
        invoke(&mut realm, &pattern, key.clone(), &[s("a")]),
        "RangeError: getter failed"
    );
    let pattern = regexp(&mut realm, "a", "g");
    let constructor = Value::Object(realm.object());
    ops::set(
        &mut realm,
        &pattern,
        &Key::name("constructor"),
        constructor.clone(),
    )
    .unwrap();
    let species = symbol(&realm, "species");
    getter(&mut realm, &constructor, species.clone());
    fails!(
        realm,
        invoke(&mut realm, &pattern, key.clone(), &[s("a")]),
        "RangeError: getter failed"
    );
    let failing = function(&realm, true, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "construct failed"))
    });
    let constructor = Value::Object(realm.object());
    ops::set(&mut realm, &constructor, &species, failing).unwrap();
    ops::set(&mut realm, &pattern, &Key::name("constructor"), constructor).unwrap();
    fails!(
        realm,
        invoke(&mut realm, &pattern, key, &[s("a")]),
        "RangeError: construct failed"
    );
    let split = symbol(&realm, "split");
    fails!(
        realm,
        invoke(&mut realm, &pattern, split, &[s("a")]),
        "RangeError: construct failed"
    );
    // Named captures whose getter throws during substitution.
    let named = regexp(&mut realm, "a", "");
    let exec = function(&realm, false, |realm, _, _| {
        let groups = realm.object();
        let thrower = realm.function("get", 0, false, |realm, _, _| {
            Err(realm.throw(ErrorKind::Range, "group failed"))
        });
        groups.define(
            Key::name("x"),
            Property {
                slot: Slot::Accessor {
                    get: Some(thrower),
                    set: None,
                },
                enumerable: true,
                configurable: true,
            },
        );
        let result = realm.array(vec![s("a")]);
        result.define(Key::name("index"), Property::data(n(0.0)));
        result.define(Key::name("groups"), Property::data(Value::Object(groups)));
        Ok(Value::Object(result))
    });
    ops::set(&mut realm, &named, &Key::name("exec"), exec).unwrap();
    fails!(
        realm,
        string(&mut realm, "a", "replace", &[named, s("$<x>")]),
        "RangeError: group failed"
    );
    // Searching restores a lastIndex the search changed.
    let global = regexp(&mut realm, "c", "g");
    check!(
        realm,
        string(&mut realm, "abc", "search", std::slice::from_ref(&global)),
        "2"
    );
    check!(realm, ops::get(&mut realm, &global, &last_index_key()), "0");
    // Debug output names the kinds.
    let debug = format!("{:?}", global.as_object().unwrap().data().kind);
    assert_eq!(debug, "RegExp(/c/g)");
    let digits = regexp(&mut realm, "\\d", "g");
    let iterator = string(&mut realm, "1", "matchAll", &[digits]).unwrap();
    let debug = format!("{:?}", iterator.as_object().unwrap().data().kind);
    assert_eq!(debug, "RegExpStringIterator");
}

#[test]
fn literal_sites_reuse_their_strings_but_check_reused_buffers() {
    let mut realm = Realm::new();
    // One buffer holding different text at different times, as a caller's
    // reused allocation does: each text gets its own pattern.
    let mut buffer = [0x61u16];
    let first = literal(&mut realm, &buffer, "").unwrap();
    buffer[0] = 0x62;
    let second = literal(&mut realm, &buffer, "").unwrap();
    check!(realm, Ok(first), "/a/");
    check!(realm, Ok(second), "/b/");
    // A site whose text stays, as a compiled literal's, shares its strings
    // and pattern every time.
    static SITE: [u16; 1] = [0x63];
    let first = literal(&mut realm, &SITE, "i").unwrap();
    let again = literal(&mut realm, &SITE, "i").unwrap();
    let (a, b) = (data_of(&first).unwrap(), data_of(&again).unwrap());
    assert!(Rc::ptr_eq(&a.pattern, &b.pattern));
    assert!(a.source.ptr_eq(&b.source) && a.flags.ptr_eq(&b.flags));
    // Sites past the limit start the table again.
    let sources: Vec<Vec<u16>> = (0..CACHE_LIMIT + 1)
        .map(|index| format!("y{index}").encode_utf16().collect())
        .collect();
    for source in &sources {
        literal(&mut realm, source, "").unwrap();
    }
    assert!(realm.patterns.literal_count <= CACHE_LIMIT);
}
