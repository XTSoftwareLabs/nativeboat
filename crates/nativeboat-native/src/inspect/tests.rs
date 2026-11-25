//! Expected texts are Node 22's output for the same values.

use super::*;

#[test]
fn native_collections_show_entries_properties_cycles_and_limits() {
    let mut realm = Realm::new();
    let mut collections = Vec::new();
    for name in ["Map", "Set"] {
        let global = object(&realm.global);
        let constructor = ops::get(&mut realm, &global, &Key::name(name)).unwrap();
        let collection = ops::construct(&mut realm, &constructor, &[]).unwrap();
        assert_eq!(show(&mut realm, &collection), format!("{name}(0) {{}}"));
        let method = ops::get(
            &mut realm,
            &collection,
            &Key::name(if name == "Map" { "set" } else { "add" }),
        )
        .unwrap();
        let nested = realm.object();
        set(&nested, "value", n(2.0));
        if name == "Map" {
            ops::call(&mut realm, &method, collection.clone(), &[s("a"), n(1.0)]).unwrap();
            ops::call(
                &mut realm,
                &method,
                collection.clone(),
                &[s("b"), object(&nested)],
            )
            .unwrap();
            assert_eq!(
                show(&mut realm, &collection),
                "Map(2) { 'a' => 1, 'b' => { value: 2 } }"
            );
        } else {
            ops::call(&mut realm, &method, collection.clone(), &[s("x")]).unwrap();
            ops::call(&mut realm, &method, collection.clone(), &[object(&nested)]).unwrap();
            assert_eq!(
                show(&mut realm, &collection),
                "Set(2) { 'x', { value: 2 } }"
            );
        }
        let parent = realm.object();
        set(&parent, "value", collection.clone());
        assert_eq!(
            inspect(&mut realm, &object(&parent), 0).unwrap(),
            format!("{{ value: [{name}] }}")
        );
        let getter = realm.function("get size", 0, false, |realm, _, _| {
            Err(realm.throw(ErrorKind::Error, "size getter"))
        });
        ops::define_accessor(
            collection.as_object().unwrap(),
            Key::name("size"),
            Some(getter),
            None,
        );
        let shown = show(&mut realm, &collection);
        assert!(shown.ends_with("size: [Getter] }"), "{shown}");
        collection.as_object().unwrap().delete(&Key::name("size"));
        collections.push((collection, method));
    }
    let (map, method) = &collections[0];
    ops::call(&mut realm, method, map.clone(), &[map.clone(), map.clone()]).unwrap();
    assert_eq!(
        show(&mut realm, map),
        "<ref *1> Map(3) {\n  'a' => 1,\n  'b' => { value: 2 },\n  [Circular *1] => [Circular *1]\n}"
    );
    let clear = ops::get(&mut realm, map, &Key::name("clear")).unwrap();
    ops::call(&mut realm, &clear, map.clone(), &[]).unwrap();
    for index in 0..101 {
        ops::call(
            &mut realm,
            method,
            map.clone(),
            &[n(index as f64), n(index as f64)],
        )
        .unwrap();
    }
    let shown = show(&mut realm, map);
    assert!(shown.starts_with("Map(101) {\n"));
    assert!(shown.ends_with("... 1 more item\n}"));
    assert!(!shown.contains("100 => 100"));
    let (set_value, _) = &collections[1];
    set_value.as_object().unwrap().set_prototype(None);
    assert_eq!(
        show(&mut realm, set_value),
        "[Set(2): null prototype] { 'x', { value: 2 } }"
    );
}

#[test]
fn inspection_bounds_deep_graphs_and_tolerates_reentrant_property_mutation() {
    let mut realm = Realm::new();
    let mut value = n(1.0);
    for _ in 0..65 {
        let object = realm.object();
        set(&object, "next", value);
        value = Value::Object(object);
    }
    assert!(inspect(&mut realm, &value, usize::MAX).is_err());
    assert_eq!(show(&mut realm, &n(2.0)), "2");
    let prototype = realm.object();
    let removed = prototype.clone();
    let method = realm.function("custom", 0, false, |_, _, _| Ok(Value::str("custom")));
    let getter = realm.function("get toString", 0, false, move |_, _, _| {
        removed.delete(&Key::name("toString"));
        removed.set_prototype(None);
        Ok(Value::Object(method.clone()))
    });
    prototype.define(
        Key::name("toString"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let child = Object::new(Some(prototype), Kind::Ordinary);
    // The first lookup deletes its inherited holder. Formatting must not
    // panic while searching for the already removed method.
    assert!(format(&mut realm, &[s("%s"), object(&child)]).is_err());
    let unrelated = realm.object();
    let throwing = realm.function("toJSON", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "a circular structure elsewhere"))
    });
    set(&unrelated, "toJSON", object(&throwing));
    assert!(format(&mut realm, &[s("%j"), object(&unrelated)]).is_err());
}
use crate::object::Object;
use crate::realm::ErrorKind;
use crate::symbol::Symbol;

#[test]
fn inspection_formats_wide_objects_multiline_function_names_and_unnamed_constructors() {
    let mut realm = Realm::new();
    let wide = realm.object();
    for index in 0..40 {
        set(&wide, &format!("p{index}"), n(index as f64));
    }
    let shown = show(&mut realm, &object(&wide));
    assert!(shown.starts_with("{\n  p0: 0,"));
    assert!(shown.ends_with("\n  p39: 39\n}"));
    let multiline = realm.function("a\nb", 0, false, |_, _, _| Ok(Value::Undefined));
    let holder = realm.object();
    set(&holder, "callback", object(&multiline));
    assert_eq!(
        show(&mut realm, &object(&holder)),
        "{\n  callback: [Function: a\nb]\n}"
    );
    for name in [n(1.0), s("")] {
        let (constructor, prototype) = class(&mut realm, "Name", None);
        hide(&constructor, Key::name("name"), name);
        let instance = Object::new(Some(prototype), Kind::Ordinary);
        set(&instance, "a", n(1.0));
        assert_eq!(show(&mut realm, &object(&instance)), "{ a: 1 }");
    }
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn object(object: &Object) -> Value {
    Value::Object(object.clone())
}

/// A value from JSON text.
fn parse(realm: &mut Realm, source: &str) -> Value {
    let global = object(&realm.global);
    let json = ops::get(realm, &global, &Key::name("JSON")).unwrap();
    let parse = ops::get(realm, &json, &Key::name("parse")).unwrap();
    ops::call(realm, &parse, json, &[s(source)]).unwrap()
}

/// The value at a path of property names.
fn path(realm: &mut Realm, value: &Value, names: &[&str]) -> Value {
    let mut value = value.clone();
    for name in names {
        value = ops::get(realm, &value, &Key::name(name)).unwrap();
    }
    value
}

fn show(realm: &mut Realm, value: &Value) -> String {
    inspect(realm, value, DEPTH).unwrap()
}

fn json(realm: &mut Realm, source: &str) -> String {
    let value = parse(realm, source);
    show(realm, &value)
}

fn fmt(realm: &mut Realm, args: &[Value]) -> String {
    format(realm, args).unwrap()
}

fn set(target: &Object, key: &str, value: Value) {
    target.define(Key::name(key), Property::data(value));
}

fn hide(target: &Object, key: Key, value: Value) {
    target.define(key, Property::hidden(value));
}

/// A class constructor and its prototype.
fn class(realm: &mut Realm, name: &str, parent: Option<&Object>) -> (Object, Object) {
    let parent_prototype = match parent {
        Some(parent) => own_data(parent, "prototype")
            .unwrap()
            .as_object()
            .unwrap()
            .clone(),
        None => realm.object_prototype.clone(),
    };
    let prototype = Object::new(Some(parent_prototype), Kind::Ordinary);
    let parent = parent.cloned().unwrap_or(realm.function_prototype.clone());
    let constructor =
        crate::class::create(realm, name, 0, &prototype, parent, |realm, _, target, _| {
            crate::class::create_this(realm, &target)
        });
    (constructor, prototype)
}

fn array_with(realm: &Realm, elements: Vec<Option<Value>>) -> Object {
    Object::new(
        Some(realm.array_prototype.clone()),
        Kind::Array(crate::object::ArrayData::new(elements)),
    )
}

fn numbers(realm: &Realm, values: impl Iterator<Item = f64>) -> Value {
    Value::Object(realm.array(values.map(Value::Number).collect()))
}

#[test]
fn primitives_print_as_node_prints_them() {
    let mut realm = Realm::new();
    assert_eq!(quote(&[39, 34, 36, 123, 113, 125]), "'\\'\"${q}'");
    let symbol = Value::Symbol(Symbol::new(Some("q".into())));
    let values = [
        Value::Undefined,
        Value::Null,
        Value::Boolean(true),
        n(-0.0),
        n(1.5),
        n(f64::NAN),
        symbol,
    ];
    let shown: Vec<String> = values.iter().map(|value| show(&mut realm, value)).collect();
    assert_eq!(
        shown,
        ["undefined", "null", "true", "-0", "1.5", "NaN", "Symbol(q)"]
    );
    assert_eq!(
        json(
            &mut realm,
            r#"{"s": "it's", "t": "say \"hi\"", "u": "both ' \" `", "v": "\u0001\u007f\u000b\n"}"#
        ),
        r#"{ s: "it's", t: 'say "hi"', u: 'both \' " `', v: '\x01\x7F\x0B\n' }"#
    );
    assert_eq!(
        json(&mut realm, r#"["'\"", "\b\t\f\r\\"]"#),
        r#"[ `'"`, '\b\t\f\r\\' ]"#
    );
    let lone = Value::String(JsStr::from_units(&[0xd800, 0x61]));
    assert_eq!(show(&mut realm, &lone), "'\\ud800a'");
}

#[test]
fn long_strings_break_after_line_feeds_and_stop_at_the_limit() {
    let mut realm = Realm::new();
    let long = "x".repeat(100);
    assert_eq!(
        json(&mut realm, &format!(r#"{{"long": "{long}"}}"#)),
        format!("{{\n  long: '{long}'\n}}")
    );
    assert_eq!(
        json(
            &mut realm,
            r#"{"long": "line one is long enough\nline two is long enough too, really long enough to break"}"#
        ),
        "{\n  long: 'line one is long enough\\n' +\n    'line two is long enough too, really long enough to break'\n}"
    );
    let cut = show(&mut realm, &s(&"a".repeat(10_002)));
    assert!(cut.ends_with("aaaa'... 2 more characters"), "{cut}");
    let cut = show(&mut realm, &s(&"a".repeat(10_001)));
    assert!(cut.ends_with("aaaa'... 1 more character"), "{cut}");
}

#[test]
fn objects_nest_to_the_depth_limit() {
    let mut realm = Realm::new();
    assert_eq!(
        json(&mut realm, r#"{"a":{"b":{"c":{"d":{}}}}}"#),
        "{ a: { b: { c: [Object] } } }"
    );
    assert_eq!(
        json(&mut realm, r#"{"a":{"b":{"c":[]}}}"#),
        "{ a: { b: { c: [] } } }"
    );
    assert_eq!(
        json(&mut realm, r#"{"a":{"b":{"c":[1]}}}"#),
        "{ a: { b: { c: [Array] } } }"
    );
    assert_eq!(json(&mut realm, "[[[[1]]]]"), "[ [ [ [Array] ] ] ]");
    assert_eq!(json(&mut realm, "{}"), "{}");
    assert_eq!(
        json(
            &mut realm,
            r#"{"a":1,"b":"two","c":[1,2,{"d":3}],"e":{"f":{"g":{"h":1}}},"longer_property_name":"some longer string value"}"#
        ),
        "{\n  a: 1,\n  b: 'two',\n  c: [ 1, 2, { d: 3 } ],\n  e: { f: { g: [Object] } },\n  longer_property_name: 'some longer string value'\n}"
    );
    // Objects nesting three levels or more inside one break its lines.
    let deep = parse(&mut realm, r#"{"a":{"b":{"c":{"d":{"e":{"f":1}}}}}}"#);
    assert_eq!(
        inspect(&mut realm, &deep, 4).unwrap(),
        "{\n  a: {\n    b: { c: { d: { e: [Object] } } }\n  }\n}"
    );
    assert_eq!(
        inspect(&mut realm, &deep, usize::MAX).unwrap(),
        "{\n  a: {\n    b: {\n      c: { d: { e: { f: 1 } } }\n    }\n  }\n}"
    );
    assert_eq!(
        json(
            &mut realm,
            r#"{"$x": 1, "_y": 2, "a-b": 3, "é": 4, "0": 5, "": 6, "a1": 7}"#
        ),
        "{ '0': 5, '$x': 1, _y: 2, 'a-b': 3, 'é': 4, '': 6, a1: 7 }"
    );
    let keyed = realm.object();
    hide(&keyed, Key::name("hidden"), n(1.0));
    keyed.define(
        Key::Symbol(Symbol::new(Some("w".into()))),
        Property::data(n(1.0)),
    );
    set(&keyed, "u", Value::Undefined);
    let getter = realm.function("g", 0, false, |_, _, _| Ok(Value::Undefined));
    for (name, get, put) in [("g", true, false), ("s", false, true), ("gs", true, true)] {
        keyed.define(
            Key::name(name),
            Property {
                slot: Slot::Accessor {
                    get: get.then(|| getter.clone()),
                    set: put.then(|| getter.clone()),
                },
                enumerable: true,
                configurable: true,
            },
        );
    }
    assert_eq!(
        show(&mut realm, &object(&keyed)),
        "{\n  u: undefined,\n  g: [Getter],\n  s: [Setter],\n  gs: [Getter/Setter],\n  [Symbol(w)]: 1\n}"
    );
}

#[test]
fn arrays_show_holes_limits_and_columns() {
    let mut realm = Realm::new();
    let holes = array_with(&realm, vec![None, Some(n(1.0)), None, None, Some(n(2.0))]);
    assert_eq!(
        show(&mut realm, &object(&holes)),
        "[ <1 empty item>, 1, <2 empty items>, 2 ]"
    );
    let empty = array_with(&realm, vec![None; 5]);
    assert_eq!(show(&mut realm, &object(&empty)), "[ <5 empty items> ]");
    let extra = realm.array(vec![n(1.0), n(2.0)]);
    set(&extra, "extra", s("x"));
    assert_eq!(show(&mut realm, &object(&extra)), "[ 1, 2, extra: 'x' ]");
    let empty = Value::Object(realm.array(vec![]));
    assert_eq!(show(&mut realm, &empty), "[]");
    let seven = numbers(&realm, (1..=7).map(f64::from));
    assert_eq!(show(&mut realm, &seven), "[\n  1, 2, 3, 4,\n  5, 6, 7\n]");
    let thirty = numbers(&realm, (0..30).map(|index| f64::from(index * 13)));
    assert_eq!(
        show(&mut realm, &thirty),
        "[\n    0,  13,  26,  39,  52,  65,  78,\n   91, 104, 117, 130, 143, 156, 169,\n  182, 195, 208, 221, 234, 247, 260,\n  273, 286, 299, 312, 325, 338, 351,\n  364, 377\n]"
    );
    let mixed = numbers(
        &realm,
        [1.5, -0.0, f64::NAN, 10.0, 200.0, 3000.0, 40000.0].into_iter(),
    );
    assert_eq!(
        show(&mut realm, &mixed),
        "[\n    1.5,  -0,  NaN,\n     10, 200, 3000,\n  40000\n]"
    );
    assert_eq!(
        json(
            &mut realm,
            r#"["a","bb","ccc","dddd","eeeee","ffffff","g"]"#
        ),
        "[ 'a', 'bb', 'ccc', 'dddd', 'eeeee', 'ffffff', 'g' ]"
    );
    assert_eq!(
        json(&mut realm, r#"[1,2,3,4,5,6,"x"]"#),
        "[\n  1, 2, 3,   4,\n  5, 6, 'x'\n]"
    );
    let keyed = numbers(&realm, (1..=7).map(f64::from));
    set(keyed.as_object().unwrap(), "k", n(1.0));
    assert_eq!(
        show(&mut realm, &keyed),
        "[\n  1, 2, 3, 4,\n  5, 6, 7, k: 1\n]"
    );
    let zeros = numbers(&realm, (0..120).map(|_| 0.0));
    let shown = show(&mut realm, &zeros);
    let row = "  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,\n";
    assert_eq!(
        shown,
        format!("[\n{}  0, 0, 0, 0,\n  ... 20 more items\n]", row.repeat(8))
    );
    // Entries too long for three columns, or for more than one, are not grouped.
    let wide = format!(
        r#"["{0}","{0}","{0}","{0}","{0}","{0}","{0}"]"#,
        "x".repeat(30)
    );
    assert_eq!(json(&mut realm, &wide).lines().count(), 9);
    let medium = format!(
        r#"["{0}","{0}","{0}","{0}","{0}","{0}","{0}"]"#,
        "x".repeat(18)
    );
    let line = format!("  '{}'", "x".repeat(18));
    assert_eq!(
        json(&mut realm, &medium),
        format!("[\n{}\n]", [line.as_str(); 7].join(",\n"))
    );
    let long = format!(
        r#"["{0}","{0}","{0}"]"#,
        "abcdefghijklmnopqrstuvwxyzabcdefghijkl"
    );
    assert_eq!(
        json(&mut realm, &long),
        "[\n  'abcdefghijklmnopqrstuvwxyzabcdefghijkl',\n  'abcdefghijklmnopqrstuvwxyzabcdefghijkl',\n  'abcdefghijklmnopqrstuvwxyzabcdefghijkl'\n]"
    );
    // A sparse array reaching the limit on a hole.
    let sparse: Vec<Option<Value>> = (0..299)
        .map(|index| (index % 2 == 0).then(|| n(f64::from(index))))
        .collect();
    let sparse = array_with(&realm, sparse);
    let shown = show(&mut realm, &object(&sparse));
    let tail: Vec<&str> = shown.lines().rev().take(3).collect();
    assert_eq!(
        tail,
        [
            "]",
            "  ... 199 more items",
            "  <1 empty item>, 96,             <1 empty item>, 98,             <1 empty item>,"
        ]
    );
}

#[test]
fn circular_references_are_numbered() {
    let mut realm = Realm::new();
    let a = realm.object();
    set(&a, "self", object(&a));
    set(&a, "list", Value::Object(realm.array(vec![object(&a)])));
    assert_eq!(
        show(&mut realm, &object(&a)),
        "<ref *1> { self: [Circular *1], list: [ [Circular *1] ] }"
    );
    let first = realm.object();
    set(&first, "c1", object(&first));
    let second = realm.object();
    set(&second, "c1", object(&first));
    set(&second, "c2", object(&second));
    let both = realm.object();
    set(&both, "c2", object(&second));
    set(&both, "c1", object(&first));
    assert_eq!(
        show(&mut realm, &object(&both)),
        "{\n  c2: <ref *2> { c1: <ref *1> { c1: [Circular *1] }, c2: [Circular *2] },\n  c1: <ref *1> { c1: [Circular *1] }\n}"
    );
    let function = realm.function("f", 0, false, |_, _, _| Ok(Value::Undefined));
    set(&function, "me", object(&function));
    assert_eq!(
        show(&mut realm, &object(&function)),
        "<ref *1> [Function: f] { me: [Circular *1] }"
    );
}

#[test]
fn prefixes_name_constructors_and_tags() {
    let mut realm = Realm::new();
    let bare = Object::new(None, Kind::Ordinary);
    assert_eq!(
        show(&mut realm, &object(&bare)),
        "[Object: null prototype] {}"
    );
    set(&bare, "a", n(1.0));
    let holder = realm.object();
    set(&holder, "x", object(&bare));
    assert_eq!(
        show(&mut realm, &object(&holder)),
        "{ x: [Object: null prototype] { a: 1 } }"
    );
    let deep = parse(&mut realm, r#"{"a":{"b":{}}}"#);
    let b = path(&mut realm, &deep, &["a", "b"]);
    set(b.as_object().unwrap(), "c", object(&bare));
    assert_eq!(
        show(&mut realm, &deep),
        "{ a: { b: { c: [Object: null prototype] } } }"
    );
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let tagged = Object::new(None, Kind::Ordinary);
    hide(&tagged, tag.clone(), s("T"));
    assert_eq!(
        show(&mut realm, &object(&tagged)),
        "[Object: null prototype] [T] {}"
    );
    let global = object(&realm.global);
    let json = ops::get(&mut realm, &global, &Key::name("JSON")).unwrap();
    assert_eq!(show(&mut realm, &json), "Object [JSON] {}");
    let own = realm.object();
    set(&own, "a", n(1.0));
    own.define(tag.clone(), Property::data(s("T")));
    assert_eq!(
        show(&mut realm, &object(&own)),
        "{ a: 1, [Symbol(Symbol.toStringTag)]: 'T' }"
    );
    let parent = realm.object();
    hide(&parent, tag.clone(), s("T"));
    let inherited = Object::new(Some(parent), Kind::Ordinary);
    set(&inherited, "a", n(1.0));
    assert_eq!(show(&mut realm, &object(&inherited)), "Object [T] { a: 1 }");

    let (foo, foo_prototype) = class(&mut realm, "Foo", None);
    let instance = Object::new(Some(foo_prototype.clone()), Kind::Ordinary);
    set(&instance, "a", n(1.0));
    let list = Value::Object(realm.array(vec![object(&instance)]));
    assert_eq!(show(&mut realm, &list), "[ Foo { a: 1 } ]");
    let bar_instance = {
        let (_, bar_prototype) = class(&mut realm, "Bar", None);
        Object::new(Some(bar_prototype), Kind::Ordinary)
    };
    assert_eq!(show(&mut realm, &object(&bar_instance)), "Bar {}");
    let nested = parse(&mut realm, r#"{"a":{"b":{}}}"#);
    let b = path(&mut realm, &nested, &["a", "b"]);
    set(b.as_object().unwrap(), "c", object(&instance));
    assert_eq!(show(&mut realm, &nested), "{ a: { b: { c: [Foo] } } }");
    let (_, t2_prototype) = class(&mut realm, "T2", None);
    hide(&t2_prototype, tag.clone(), s("T"));
    let t2 = Object::new(Some(t2_prototype), Kind::Ordinary);
    assert_eq!(show(&mut realm, &object(&t2)), "T2 [T] {}");

    // Constructors that are unnamed, not functions or not in the chain are
    // skipped.
    let skipped = realm.object();
    let unnamed = realm.function("", 0, true, |_, _, _| Ok(Value::Undefined));
    let unrelated = realm.function("Unrelated", 0, true, |_, _, _| Ok(Value::Undefined));
    let middle = Object::new(Some(realm.object_prototype.clone()), Kind::Ordinary);
    hide(&middle, Key::name("constructor"), object(&unnamed));
    hide(&unnamed, Key::name("prototype"), object(&middle));
    let not_function = Object::new(Some(middle), Kind::Ordinary);
    hide(
        &not_function,
        Key::name("constructor"),
        object(&realm.object()),
    );
    hide(&skipped, Key::name("constructor"), object(&unrelated));
    skipped.set_prototype(Some(not_function));
    assert_eq!(show(&mut realm, &object(&skipped)), "{}");
    let getter = realm.object();
    getter.define(
        Key::name("constructor"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    assert_eq!(show(&mut realm, &object(&getter)), "{}");

    // Array subclasses show their length.
    let list_class = realm.function("L", 0, true, |_, _, _| Ok(Value::Undefined));
    let list_prototype = Object::new(Some(realm.array_prototype.clone()), Kind::Ordinary);
    hide(
        &list_prototype,
        Key::name("constructor"),
        object(&list_class),
    );
    hide(&list_class, Key::name("prototype"), object(&list_prototype));
    let sub = realm.array(vec![n(1.0), n(2.0)]);
    sub.set_prototype(Some(list_prototype));
    assert_eq!(show(&mut realm, &object(&sub)), "L(2) [ 1, 2 ]");
    let nested = parse(&mut realm, r#"{"a":{"b":{}}}"#);
    let b = path(&mut realm, &nested, &["a", "b"]);
    set(b.as_object().unwrap(), "c", object(&sub));
    assert_eq!(show(&mut realm, &nested), "{ a: { b: { c: [L] } } }");
    let _ = foo;
}

#[test]
fn functions_and_classes_show_their_names() {
    let mut realm = Realm::new();
    let named = realm.function("named", 0, false, |_, _, _| Ok(Value::Undefined));
    assert_eq!(show(&mut realm, &object(&named)), "[Function: named]");
    let anonymous = realm.function("", 0, false, |_, _, _| Ok(Value::Undefined));
    assert_eq!(
        show(&mut realm, &object(&anonymous)),
        "[Function (anonymous)]"
    );
    set(&named, "prop", n(1.0));
    assert_eq!(
        show(&mut realm, &object(&named)),
        "[Function: named] { prop: 1 }"
    );
    let numbered = realm.function("x", 0, false, |_, _, _| Ok(Value::Undefined));
    hide(&numbered, Key::name("name"), n(3.0));
    assert_eq!(show(&mut realm, &object(&numbered)), "[Function: 3]");
    let orphan = realm.function("f", 0, false, |_, _, _| Ok(Value::Undefined));
    orphan.set_prototype(None);
    assert_eq!(
        show(&mut realm, &object(&orphan)),
        "[Function (null prototype): f]"
    );
    let (foo, foo_prototype) = class(&mut realm, "Foo", None);
    let odd = realm.function("g", 0, false, |_, _, _| Ok(Value::Undefined));
    odd.set_prototype(Some(foo_prototype.clone()));
    assert_eq!(show(&mut realm, &object(&odd)), "[Function: g] Foo");
    let tagged = realm.function("h", 0, false, |_, _, _| Ok(Value::Undefined));
    hide(
        &tagged,
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        s("Q"),
    );
    assert_eq!(show(&mut realm, &object(&tagged)), "[Function: h] [Q]");

    assert_eq!(show(&mut realm, &object(&foo)), "[class Foo]");
    let (bar, _) = class(&mut realm, "Bar", Some(&foo));
    assert_eq!(show(&mut realm, &object(&bar)), "[class Bar extends Foo]");
    let (anonymous, _) = class(&mut realm, "", None);
    assert_eq!(show(&mut realm, &object(&anonymous)), "[class (anonymous)]");
    anonymous.delete(&Key::name("name"));
    assert_eq!(show(&mut realm, &object(&anonymous)), "[class (anonymous)]");
    let (rooted, _) = class(&mut realm, "Rooted", None);
    rooted.set_prototype(None);
    assert_eq!(
        show(&mut realm, &object(&rooted)),
        "[class Rooted extends [null prototype]]"
    );
    let (odd_class, _) = class(&mut realm, "Odd", None);
    odd_class.set_prototype(Some(foo_prototype));
    hide(
        &odd_class,
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        s("Q"),
    );
    assert_eq!(
        show(&mut realm, &object(&odd_class)),
        "[class Odd [Foo] [Q]]"
    );
    let (unnamed_parent, _) = class(&mut realm, "Child", None);
    let parent = realm.object();
    hide(&parent, Key::name("name"), n(1.0));
    unnamed_parent.set_prototype(Some(parent));
    assert_eq!(
        show(&mut realm, &object(&unnamed_parent)),
        "[class Child [Object] extends 1]"
    );
    // Built-in constructors are functions, not classes.
    let global = object(&realm.global);
    let array = ops::get(&mut realm, &global, &Key::name("Array")).unwrap();
    assert_eq!(show(&mut realm, &array), "[Function: Array]");
}

#[test]
fn wrappers_errors_and_arguments_have_their_own_forms() {
    let mut realm = Realm::new();
    let wrapped = [
        Wrapped::Number(-0.0),
        Wrapped::String("ab".into()),
        Wrapped::Boolean(false),
    ];
    let shown: Vec<String> = wrapped
        .into_iter()
        .map(|value| {
            let boxed = realm.primitive_object(value);
            show(&mut realm, &object(&boxed))
        })
        .collect();
    assert_eq!(
        shown,
        ["[Number: -0]", "[String: 'ab']", "[Boolean: false]"]
    );
    let symbol = Object::new(
        Some(realm.symbol_prototype.clone()),
        Kind::SymbolObject(Symbol::new(Some("s".into()))),
    );
    assert_eq!(show(&mut realm, &object(&symbol)), "[Symbol: Symbol(s)]");
    let keyed = realm.primitive_object(Wrapped::String("ab".into()));
    set(&keyed, "x", n(1.0));
    assert_eq!(show(&mut realm, &object(&keyed)), "[String: 'ab'] { x: 1 }");
    let orphan = realm.primitive_object(Wrapped::Number(1.0));
    orphan.set_prototype(None);
    assert_eq!(
        show(&mut realm, &object(&orphan)),
        "[Number (null prototype): 1]"
    );
    let (_, foo_prototype) = class(&mut realm, "Foo", None);
    let adopted = realm.primitive_object(Wrapped::Number(1.0));
    adopted.set_prototype(Some(foo_prototype));
    assert_eq!(show(&mut realm, &object(&adopted)), "[Number (Foo): 1]");
    let tagged = realm.primitive_object(Wrapped::Boolean(true));
    hide(
        &tagged,
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        s("W"),
    );
    assert_eq!(show(&mut realm, &object(&tagged)), "[Boolean: true] [W]");

    let error = realm.error(ErrorKind::Error, "boom");
    assert_eq!(show(&mut realm, &object(&error)), "[Error: boom]");
    set(&error, "code", s("X"));
    assert_eq!(
        show(&mut realm, &object(&error)),
        "[Error: boom] { code: 'X' }"
    );
    let type_error = realm.error(ErrorKind::Type, "t");
    assert_eq!(show(&mut realm, &object(&type_error)), "[TypeError: t]");

    let none = realm.arguments(&[]);
    assert_eq!(show(&mut realm, &object(&none)), "[Arguments] {}");
    let some = realm.arguments(&[n(1.0), s("a")]);
    assert_eq!(
        show(&mut realm, &object(&some)),
        "[Arguments] { '0': 1, '1': 'a' }"
    );
    let adopted = realm.arguments(&[n(1.0)]);
    adopted.set_prototype(None);
    assert_eq!(
        show(&mut realm, &object(&adopted)),
        "[Object: null prototype] { '0': 1 }"
    );
}

#[test]
fn format_replaces_placeholders_like_node() {
    let mut realm = Realm::new();
    let a = parse(&mut realm, r#"{"a":1}"#);
    let one = parse(&mut realm, "[1]");
    let b = parse(&mut realm, r#"{"b":2}"#);
    let args = [
        s("%s|%d|%i|%f|%j|%o|%O|%c|%%|%x"),
        s("str"),
        s("42.5"),
        s("42.9px"),
        s("3.5e1z"),
        a,
        one,
        b,
        s("css"),
    ];
    assert_eq!(
        fmt(&mut realm, &args),
        r#"str|42.5|42|35|{"a":1}|[ 1 ]|{ b: 2 }||%|%x"#
    );
    let nested = parse(&mut realm, r#"{"a":{"b":1}}"#);
    assert_eq!(
        fmt(&mut realm, &[s("%s"), nested, s("%s")]),
        "{ a: [Object] } %s"
    );
    assert_eq!(fmt(&mut realm, &[s("100%%")]), "100%%");
    assert_eq!(fmt(&mut realm, &[s("100%%"), n(1.0)]), "100% 1");
    assert_eq!(
        fmt(&mut realm, &[s("a"), s("b"), n(1.0), s("c")]),
        "a b 1 c"
    );
    assert_eq!(fmt(&mut realm, &[s("%s %s"), n(1.0)]), "1 %s");
    assert_eq!(fmt(&mut realm, &[n(1.0), s("%s"), n(2.0)]), "1 %s 2");
    assert_eq!(
        fmt(&mut realm, &[s("%s"), n(-0.0), s("%d"), n(-0.0)]),
        "-0 %d -0"
    );
    let empty = Value::Object(realm.object());
    assert_eq!(fmt(&mut realm, &[s("%d"), empty]), "NaN");
    let symbol = Value::Symbol(Symbol::new(None));
    for placeholder in ["%d", "%i", "%f"] {
        assert_eq!(fmt(&mut realm, &[s(placeholder), symbol.clone()]), "NaN");
    }
    let z = Value::Symbol(Symbol::new(Some("z".into())));
    assert_eq!(
        fmt(
            &mut realm,
            &[s("%s %s %s"), Value::Null, Value::Undefined, z]
        ),
        "null undefined Symbol(z)"
    );
    assert_eq!(fmt(&mut realm, &[s("%é"), n(1.0)]), "%é 1");
    assert_eq!(fmt(&mut realm, &[s("a%"), n(1.0)]), "a% 1");
    assert_eq!(fmt(&mut realm, &[s("%s"), s("x"), s("%%")]), "x %%");
    assert_eq!(fmt(&mut realm, &[s("%j"), Value::Undefined]), "undefined");
    assert_eq!(fmt(&mut realm, &[]), "");
}

#[test]
fn format_s_inspects_objects_with_built_in_to_string() {
    let mut realm = Realm::new();
    let (_, foo_prototype) = class(&mut realm, "Foo", None);
    let instance = Object::new(Some(foo_prototype), Kind::Ordinary);
    set(&instance, "a", n(1.0));
    assert_eq!(
        fmt(&mut realm, &[s("%s"), object(&instance)]),
        "Foo { a: 1 }"
    );
    let nested = parse(&mut realm, "[1,[2,[3]]]");
    assert_eq!(fmt(&mut realm, &[s("%s"), nested]), "[ 1, [Array] ]");
    let bare = Object::new(None, Kind::Ordinary);
    assert_eq!(
        fmt(&mut realm, &[s("%s"), object(&bare)]),
        "[Object: null prototype] {}"
    );
    let error = realm.error(ErrorKind::Error, "boom");
    assert_eq!(fmt(&mut realm, &[s("%s"), object(&error)]), "[Error: boom]");

    let custom = realm.function("toString", 0, false, |_, _, _| Ok(Value::str("custom")));
    let own = realm.object();
    set(&own, "toString", object(&custom));
    assert_eq!(fmt(&mut realm, &[s("%s"), object(&own)]), "custom");
    // An inherited method from an object without a constructor.
    let child = Object::new(Some(own.clone()), Kind::Ordinary);
    assert_eq!(fmt(&mut realm, &[s("%s"), object(&child)]), "custom");
    // An inherited method from a class that is not a global built-in.
    for name in ["lower", "Upper"] {
        let (_, prototype) = class(&mut realm, name, None);
        set(&prototype, "toString", object(&custom));
        let instance = Object::new(Some(prototype), Kind::Ordinary);
        assert_eq!(fmt(&mut realm, &[s("%s"), object(&instance)]), "custom");
    }
    let (unnamed, prototype) = class(&mut realm, "X", None);
    unnamed.delete(&Key::name("name"));
    set(&prototype, "toString", object(&custom));
    let instance = Object::new(Some(prototype), Kind::Ordinary);
    assert_eq!(fmt(&mut realm, &[s("%s"), object(&instance)]), "custom");
    // Functions convert with their own toString.
    let function = realm.function("f", 0, false, |_, _, _| Ok(Value::Undefined));
    set(&function, "toString", object(&custom));
    assert_eq!(fmt(&mut realm, &[s("%s"), object(&function)]), "custom");
}

#[test]
fn format_j_reports_circular_structures_and_rethrows_other_errors() {
    let mut realm = Realm::new();
    let circular = realm.object();
    set(&circular, "self", object(&circular));
    assert_eq!(fmt(&mut realm, &[s("%j"), object(&circular)]), "[Circular]");
    let failing = realm.object();
    let thrower = realm.function("toJSON", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "no"))
    });
    set(&failing, "toJSON", object(&thrower));
    assert!(format(&mut realm, &[s("%j"), object(&failing)]).is_err());
    let primitive = realm.function("toJSON", 0, false, |_, _, _| Err(Thrown(Value::str("raw"))));
    set(&failing, "toJSON", object(&primitive));
    assert!(format(&mut realm, &[s("%j"), object(&failing)]).is_err());
}

#[test]
fn strip_vt_control_characters_handles_ansi_and_preserves_unterminated_sequences() {
    for (input, expected) in [
        ("plain text", "plain text"),
        ("a\u{1b}[31mred\u{1b}[0mz", "aredz"),
        ("a\u{9b}32mgreen\u{9b}0mz", "agreenz"),
        ("a\u{1b}]0;title\u{7}z", "az"),
        ("a\u{1b}]0;title\u{1b}\\z", "az"),
        ("a\u{90}payload\u{9c}z", "az"),
        ("a\u{1b}c z", "a z"),
        ("a\u{1b}<z", "a\u{1b}<z"),
        ("a\u{1b}", "a\u{1b}"),
        ("a\u{1b}[31", "a\u{1b}[31"),
        ("a\u{1b}]unfinished", "a\u{1b}]unfinished"),
    ] {
        assert_eq!(
            strip_vt_control_characters(&JsStr::from(input)).to_rust_lossy(),
            expected,
            "input {input:?}"
        );
    }
    let input = JsStr::from_units(&[
        0xd800,
        0x1b,
        b'[' as u16,
        b'3' as u16,
        b'1' as u16,
        b'm' as u16,
    ]);
    let output = strip_vt_control_characters(&input);
    assert_eq!(output.units(), &[0xd800]);
}

#[test]
fn errors_show_hidden_causes_and_aggregated_errors() {
    let mut realm = Realm::new();
    let error = realm.error(ErrorKind::Error, "x");
    error.define(Key::name("cause"), Property::hidden(s("y")));
    assert_eq!(
        show(&mut realm, &object(&error)),
        "[Error: x] { [cause]: 'y' }"
    );
    let listed = realm.error(ErrorKind::Error, "z");
    listed.define(Key::name("errors"), Property::hidden(n(1.0)));
    assert_eq!(show(&mut realm, &object(&listed)), "[Error: z]");
}
