//! Expected values are Node 22's results.

use super::*;

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

fn new(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let constructor = global(realm, name);
    ops::construct(realm, &constructor, args)
}

fn text(realm: &mut Realm, result: Completion) -> String {
    match result {
        Ok(value) => ops::to_string(realm, &value).unwrap().to_rust_lossy(),
        Err(thrown) => format!(
            "throws {}",
            ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
        ),
    }
}

fn call(realm: &mut Realm, target: &Value, method: &str, args: &[Value]) -> String {
    let function = ops::get(realm, target, &Key::name(method)).unwrap();
    let result = ops::call(realm, &function, target.clone(), args);
    text(realm, result)
}

fn array(realm: &Realm, values: &[f64]) -> Value {
    Value::Object(realm.array(values.iter().copied().map(n).collect()))
}

fn typed(realm: &mut Realm, name: &str, values: &[f64]) -> Value {
    let values = array(realm, values);
    new(realm, name, &[values]).unwrap()
}

fn function(
    realm: &Realm,
    constructor: bool,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Value {
    Value::Object(realm.function("f", 1, constructor, code))
}

#[test]
fn elements_convert_like_node() {
    let mut realm = Realm::new();
    let cases: [(&str, &[f64], &str); 9] = [
        ("Int8Array", &[127.0, 128.0, -129.0], "127,-128,127"),
        ("Uint8Array", &[256.0, -1.0, 1.9], "0,255,1"),
        (
            "Uint8ClampedArray",
            &[0.5, 1.5, 2.4, 2.6, -1.0, 300.0, f64::NAN],
            "0,2,2,3,0,255,0",
        ),
        ("Int16Array", &[40000.0, -40000.0], "-25536,25536"),
        ("Uint16Array", &[-1.0, 65536.0], "65535,0"),
        (
            "Int32Array",
            &[2147483648.0, f64::INFINITY],
            "-2147483648,0",
        ),
        ("Uint32Array", &[-1.0, 4294967296.0], "4294967295,0"),
        ("Float32Array", &[0.1, 1e40], "0.10000000149011612,Infinity"),
        ("Float64Array", &[0.1, -0.0], "0.1,0"),
    ];
    for (name, values, expected) in cases {
        let made = typed(&mut realm, name, values);
        assert_eq!(call(&mut realm, &made, "join", &[]), expected, "{name}");
    }
    assert_eq!(primitive_number(&Value::Boolean(true)), 1.0);
    assert_eq!(primitive_number(&Value::Null), 0.0);
    assert_eq!(primitive_number(&s("7")), 7.0);
    assert!(primitive_number(&Value::Undefined).is_nan());
    for (key, expected) in [
        (Key::name("1.5"), Some(1.5)),
        (Key::name("-0"), Some(-0.0)),
        (Key::name("01"), None),
        (Key::name("x"), None),
        (Key::Symbol(realm.symbols.iterator.clone()), None),
    ] {
        assert_eq!(numeric_key(&key), expected);
    }
}

#[test]
fn integer_indexed_properties() {
    let mut realm = Realm::new();
    let made = typed(&mut realm, "Uint8Array", &[1.0, 2.0]);
    let object = made.as_object().unwrap().clone();
    // Elements are data properties; other numeric keys are never present.
    let element = object.own_property(&Key::Index(1)).unwrap();
    assert!(element.enumerable && element.configurable);
    for key in ["2", "1.5", "-0", "-1"] {
        assert!(object.own_property(&Key::name(key)).is_none(), "{key}");
    }
    assert!(!object.delete(&Key::Index(0)));
    assert!(object.delete(&Key::Index(9)));
    // Definitions write valid indexes with writable, enumerable,
    // configurable data only.
    assert!(object.define(Key::Index(0), Property::data(s("9"))));
    assert!(object.define(Key::Index(1), Property::data(n(3.0))));
    assert_eq!(call(&mut realm, &made, "join", &[]), "9,3");
    assert!(!object.define(Key::Index(5), Property::data(n(1.0))));
    assert!(!object.define(Key::Index(0), Property::hidden(n(1.0))));
    let getter = realm.function("get", 0, false, |_, _, _| Ok(Value::Undefined));
    let accessor = Property {
        slot: Slot::Accessor {
            get: Some(getter),
            set: None,
        },
        enumerable: true,
        configurable: true,
    };
    assert!(!object.define(Key::Index(0), accessor));
    let fixed = Property {
        slot: Slot::Data {
            value: n(1.0),
            writable: false,
        },
        enumerable: true,
        configurable: true,
    };
    assert!(!object.define(Key::Index(0), fixed));
    let unconfigurable = Property {
        slot: Slot::Data {
            value: n(1.0),
            writable: true,
        },
        enumerable: true,
        configurable: false,
    };
    assert!(!object.define(Key::Index(0), unconfigurable));
    // Assignment converts with ToNumber, including user valueOf.
    let valued = realm.object();
    let value_of = realm.function("valueOf", 0, false, |_, _, _| Ok(n(42.0)));
    valued.define(
        Key::name("valueOf"),
        Property::data(Value::Object(value_of)),
    );
    ops::set(&mut realm, &made, &Key::Index(1), Value::Object(valued)).unwrap();
    ops::set(&mut realm, &made, &Key::Index(7), n(1.0)).unwrap();
    assert_eq!(call(&mut realm, &made, "join", &[]), "9,42");
    // Non-numeric keys are ordinary.
    ops::set(&mut realm, &made, &Key::name("extra"), s("x")).unwrap();
    let keys: Vec<String> = object
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(keys, ["0", "1", "extra"]);
    let shown = crate::inspect::inspect(&mut realm, &made, 2).unwrap();
    assert_eq!(shown, "Uint8Array(2) [ 9, 42, extra: 'x' ]");
    let debug = format!("{:?}", object.data().kind);
    assert_eq!(debug, "Uint8Array(2)");
}

#[test]
fn construction_from_lengths_buffers_and_sources() {
    let mut realm = Realm::new();
    let errors = [
        (
            "Uint8Array",
            vec![n(-1.0)],
            "throws RangeError: Invalid typed array length: -1",
        ),
        (
            "Float64Array",
            vec![n(1e9)],
            "throws RangeError: Invalid typed array length: 1000000000",
        ),
        (
            "ArrayBuffer",
            vec![n(1e10)],
            "throws RangeError: Array buffer allocation failed",
        ),
        (
            "ArrayBuffer",
            vec![n(-1.0)],
            "throws RangeError: Invalid array buffer length",
        ),
    ];
    for (name, args, expected) in errors {
        let result = new(&mut realm, name, &args);
        assert_eq!(text(&mut realm, result), expected, "{name}");
    }
    let buffer = new(&mut realm, "ArrayBuffer", &[n(8.0)]).unwrap();
    let odd = new(&mut realm, "ArrayBuffer", &[n(3.0)]).unwrap();
    let cases = [
        (
            vec![buffer.clone(), n(1.0)],
            "throws RangeError: start offset of Uint16Array should be a multiple of 2",
        ),
        (
            vec![odd],
            "throws RangeError: byte length of Uint16Array should be a multiple of 2",
        ),
        (
            vec![buffer.clone(), n(10.0)],
            "throws RangeError: Start offset 10 is outside the bounds of the buffer",
        ),
        (
            vec![buffer.clone(), n(2.0), n(4.0)],
            "throws RangeError: Invalid typed array length: 4",
        ),
        (vec![buffer.clone(), n(2.0), n(2.0)], "0,0"),
        (vec![buffer.clone(), n(4.0)], "0,0"),
    ];
    for (args, expected) in cases {
        let result = new(&mut realm, "Uint16Array", &args);
        let result = match result {
            Ok(made) => Ok(Value::str(&call(&mut realm, &made, "join", &[]))),
            failed => failed,
        };
        assert_eq!(text(&mut realm, result), expected);
    }
    // From typed arrays, array-likes and iterables.
    let source = typed(&mut realm, "Float64Array", &[1.5, 300.0]);
    let copied = new(&mut realm, "Uint8Array", &[source]).unwrap();
    assert_eq!(call(&mut realm, &copied, "join", &[]), "1,44");
    let like = realm.object();
    like.define(Key::name("length"), Property::data(n(2.0)));
    like.define(Key::Index(1), Property::data(n(5.0)));
    let from_like = new(&mut realm, "Int8Array", &[Value::Object(like)]).unwrap();
    assert_eq!(call(&mut realm, &from_like, "join", &[]), "0,5");
    let values = array(&realm, &[7.0, 8.0]);
    let set = new(&mut realm, "Set", &[values]).unwrap();
    let from_set = new(&mut realm, "Uint8Array", &[set]).unwrap();
    assert_eq!(call(&mut realm, &from_set, "join", &[]), "7,8");
    // Subclass prototypes and calls without new.
    let constructor = global(&mut realm, "Uint8Array");
    let target = function(&realm, true, |_, this, _| Ok(this));
    let prototype = realm.object();
    ops::set(
        &mut realm,
        &target,
        &Key::name("prototype"),
        Value::Object(prototype.clone()),
    )
    .unwrap();
    let derived = ops::construct_target(&mut realm, &constructor, &[n(1.0)], &target).unwrap();
    assert_eq!(
        derived.as_object().unwrap().data().prototype,
        Some(prototype)
    );
    ops::set(&mut realm, &target, &Key::name("prototype"), n(1.0)).unwrap();
    let plain = ops::construct_target(&mut realm, &constructor, &[n(1.0)], &target).unwrap();
    assert_eq!(
        plain.as_object().unwrap().data().prototype,
        Some(realm.typed_array_prototypes[1].clone())
    );
    let throwing = realm.function("target", 0, true, |_, _, _| Ok(Value::Undefined));
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "prototype"))
    });
    throwing.define(
        Key::name("prototype"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let failed = ops::construct_target(
        &mut realm,
        &constructor,
        &[n(1.0)],
        &Value::Object(throwing),
    );
    assert_eq!(text(&mut realm, failed), "throws RangeError: prototype");
    let buffer = new(&mut realm, "ArrayBuffer", &[n(2.0)]).unwrap();
    let negative = new(&mut realm, "Uint16Array", &[buffer, n(-1.0)]);
    assert!(text(&mut realm, negative).starts_with("throws RangeError"));
    let called = ops::call(&mut realm, &constructor, Value::Undefined, &[]);
    assert_eq!(
        text(&mut realm, called),
        "throws TypeError: Constructor Uint8Array requires 'new'"
    );
    let abstract_constructor = Value::Object(
        constructor
            .as_object()
            .unwrap()
            .data()
            .prototype
            .clone()
            .unwrap(),
    );
    for result in [
        ops::call(&mut realm, &abstract_constructor, Value::Undefined, &[]),
        ops::construct(&mut realm, &abstract_constructor, &[]),
    ] {
        assert_eq!(
            text(&mut realm, result),
            "throws TypeError: Abstract class TypedArray not directly constructable"
        );
    }
}

#[test]
fn statics_and_species() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "Int16Array");
    let source = array(&realm, &[1.0, 2.0]);
    let doubled = function(&realm, false, |realm, _, args| {
        let value = ops::to_number(realm, &args[0])?;
        Ok(n(value * 2.0))
    });
    let from = call(
        &mut realm,
        &constructor,
        "from",
        &[source.clone(), doubled.clone()],
    );
    assert_eq!(from, "2,4");
    let throws = function(&realm, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "mapped"))
    });
    let mapped = call(&mut realm, &constructor, "from", &[source.clone(), throws]);
    assert_eq!(mapped, "throws RangeError: mapped");
    let like = realm.object();
    like.define(Key::name("length"), Property::data(n(1.0)));
    assert_eq!(
        call(&mut realm, &constructor, "from", &[Value::Object(like)]),
        "0"
    );
    assert_eq!(
        call(&mut realm, &constructor, "of", &[n(3.0), n(4.0)]),
        "3,4"
    );
    let not_typed = function(&realm, true, |realm, _, _| {
        Ok(Value::Object(realm.object()))
    });
    let bad = [
        (
            n(1.0),
            "from",
            vec![source.clone()],
            "throws TypeError: this is not a constructor",
        ),
        (
            n(1.0),
            "of",
            vec![],
            "throws TypeError: this is not a constructor",
        ),
        (
            constructor.clone(),
            "from",
            vec![source.clone(), n(1.0)],
            "throws TypeError: mapper is not a function",
        ),
        (
            not_typed.clone(),
            "from",
            vec![source.clone()],
            "throws TypeError: constructor did not return a TypedArray object",
        ),
        (
            not_typed.clone(),
            "of",
            vec![],
            "throws TypeError: constructor did not return a TypedArray object",
        ),
    ];
    for (this, method, args, expected) in bad {
        let function = ops::get(&mut realm, &constructor, &Key::name(method)).unwrap();
        let result = ops::call(&mut realm, &function, this, &args);
        assert_eq!(text(&mut realm, result), expected, "{method}");
    }
    // Species constructors decide the class of derived arrays.
    let made = typed(&mut realm, "Uint8Array", &[1.0, 2.0, 3.0]);
    let key = Key::name("constructor");
    let species = Key::Symbol(realm.symbols.species.clone());
    ops::set(&mut realm, &made, &key, n(1.0)).unwrap();
    assert_eq!(
        call(&mut realm, &made, "slice", &[]),
        "throws TypeError: object.constructor is not an object"
    );
    let holder = Value::Object(realm.object());
    ops::set(&mut realm, &made, &key, holder.clone()).unwrap();
    assert_eq!(call(&mut realm, &made, "slice", &[]), "1,2,3");
    ops::set(&mut realm, &holder, &species, n(1.0)).unwrap();
    assert_eq!(
        call(&mut realm, &made, "slice", &[]),
        "throws TypeError: object.constructor[Symbol.species] is not a constructor"
    );
    ops::set(&mut realm, &holder, &species, not_typed).unwrap();
    assert_eq!(
        call(&mut realm, &made, "slice", &[]),
        "throws TypeError: species constructor did not return a TypedArray object"
    );
    let small = function(&realm, true, |realm, _, _| {
        let constructor = global(realm, "Uint8Array");
        ops::construct(realm, &constructor, &[n(1.0)])
    });
    ops::set(&mut realm, &holder, &species, small).unwrap();
    assert_eq!(
        call(&mut realm, &made, "slice", &[]),
        "throws TypeError: species constructor returned a TypedArray that is too small"
    );
    ops::set(&mut realm, &made, &key, Value::Undefined).unwrap();
    assert_eq!(call(&mut realm, &made, "subarray", &[n(1.0)]), "2,3");
}

#[test]
fn prototype_methods() {
    let mut realm = Realm::new();
    let made = typed(&mut realm, "Uint8Array", &[3.0, 1.0, 2.0]);
    let identity = function(&realm, false, |_, _, args| Ok(args[0].clone()));
    let odd = function(&realm, false, |realm, _, args| {
        let value = ops::to_number(realm, &args[0])?;
        Ok(Value::Boolean(value % 2.0 == 1.0))
    });
    let descending = function(&realm, false, |realm, _, args| {
        let left = ops::to_number(realm, &args[0])?;
        let right = ops::to_number(realm, &args[1])?;
        Ok(n(right - left))
    });
    let failing = function(&realm, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "compare"))
    });
    let cases: Vec<(&str, Vec<Value>, &str)> = vec![
        ("map", vec![identity.clone()], "3,1,2"),
        ("filter", vec![odd.clone()], "3,1"),
        ("map", vec![n(1.0)], "throws TypeError: 1 is not a function"),
        (
            "filter",
            vec![n(1.0)],
            "throws TypeError: 1 is not a function",
        ),
        (
            "sort",
            vec![n(1.0)],
            "throws TypeError: The comparison function must be either a function or undefined",
        ),
        (
            "toSorted",
            vec![n(1.0)],
            "throws TypeError: The comparison function must be either a function or undefined",
        ),
        ("toSorted", vec![descending.clone()], "3,2,1"),
        ("toSorted", vec![failing], "throws RangeError: compare"),
        ("toReversed", vec![], "2,1,3"),
        ("with", vec![n(-1.0), n(9.0)], "3,1,9"),
        (
            "with",
            vec![n(3.0), n(9.0)],
            "throws RangeError: Invalid typed array index",
        ),
        ("slice", vec![n(-2.0)], "1,2"),
        ("slice", vec![n(2.0), n(1.0)], ""),
        ("subarray", vec![n(1.0), n(2.0)], "1"),
        ("at", vec![n(-1.0)], "2"),
        ("includes", vec![n(1.0)], "true"),
        ("indexOf", vec![n(2.0)], "2"),
        ("join", vec![s("-")], "3-1-2"),
        ("toString", vec![], "3,1,2"),
    ];
    for (method, args, expected) in cases {
        assert_eq!(call(&mut realm, &made, method, &args), expected, "{method}");
    }
    let values = typed(
        &mut realm,
        "Float64Array",
        &[3.0, f64::NAN, -0.0, 0.0, -1.0, f64::NAN],
    );
    assert_eq!(call(&mut realm, &values, "sort", &[]), "-1,0,0,3,NaN,NaN");
    let sorted = typed(&mut realm, "Uint8Array", &[1.0, 3.0, 2.0]);
    assert_eq!(call(&mut realm, &sorted, "sort", &[descending]), "3,2,1");
    let filled = typed(&mut realm, "Int32Array", &[0.0, 0.0, 0.0]);
    assert_eq!(
        call(&mut realm, &filled, "fill", &[n(7.0), n(1.0)]),
        "0,7,7"
    );
    assert_eq!(
        call(&mut realm, &filled, "fill", &[n(5.0), n(2.0), n(1.0)]),
        "0,7,7"
    );
    let copied = typed(&mut realm, "Uint8Array", &[1.0, 2.0, 3.0, 4.0, 5.0]);
    assert_eq!(
        call(&mut realm, &copied, "copyWithin", &[n(0.0), n(3.0)]),
        "4,5,3,4,5"
    );
    assert_eq!(call(&mut realm, &copied, "reverse", &[]), "5,4,3,5,4");
    // set copies typed arrays and array-likes within bounds.
    let target = typed(&mut realm, "Uint8Array", &[0.0, 0.0, 0.0, 0.0]);
    let pair = array(&realm, &[9.0, 8.0]);
    call(&mut realm, &target, "set", &[pair.clone(), n(1.0)]);
    let single = typed(&mut realm, "Uint8Array", &[7.0]);
    call(&mut realm, &target, "set", &[single.clone(), n(3.0)]);
    assert_eq!(call(&mut realm, &target, "join", &[]), "0,9,8,7");
    for args in [
        vec![pair.clone(), n(3.0)],
        vec![single, n(4.0)],
        vec![pair, n(-1.0)],
    ] {
        assert!(call(&mut realm, &target, "set", &args).starts_with("throws RangeError"));
    }
    // Accessors and receivers.
    let window = {
        let buffer = new(&mut realm, "ArrayBuffer", &[n(8.0)]).unwrap();
        new(&mut realm, "Uint16Array", &[buffer, n(2.0), n(2.0)]).unwrap()
    };
    for (name, expected) in [("byteLength", "4"), ("byteOffset", "2"), ("length", "2")] {
        let value = ops::get(&mut realm, &window, &Key::name(name)).unwrap();
        assert_eq!(text(&mut realm, Ok(value)), expected, "{name}");
    }
    let buffer = ops::get(&mut realm, &window, &Key::name("buffer")).unwrap();
    assert!(buffer_of(&buffer).is_some());
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let prototype = Value::Object(realm.typed_array_prototype.clone());
    assert!(matches!(
        ops::get(&mut realm, &prototype, &tag).unwrap(),
        Value::Undefined
    ));
    let plain = Value::Object(realm.object());
    for method in ["join", "fill", "length"] {
        let function = match realm
            .typed_array_prototype
            .own_property(&Key::name(method))
            .unwrap()
            .slot
        {
            Slot::Data { value, .. } => value,
            Slot::Accessor { get, .. } => Value::Object(get.unwrap()),
        };
        let result = ops::call(&mut realm, &function, plain.clone(), &[]);
        assert!(
            text(&mut realm, result).contains("is not a typed array"),
            "{method}"
        );
    }
}

#[test]
fn array_buffers_and_data_views() {
    let mut realm = Realm::new();
    let buffer = new(&mut realm, "ArrayBuffer", &[n(8.0)]).unwrap();
    let constructor = global(&mut realm, "ArrayBuffer");
    let called = ops::call(&mut realm, &constructor, Value::Undefined, &[]);
    assert_eq!(
        text(&mut realm, called),
        "throws TypeError: Constructor ArrayBuffer requires 'new'"
    );
    let view = new(&mut realm, "DataView", std::slice::from_ref(&buffer)).unwrap();
    for (method, args) in [
        ("setUint16", vec![n(0.0), n(0x1234 as f64)]),
        (
            "setUint16",
            vec![n(2.0), n(0x1234 as f64), Value::Boolean(true)],
        ),
        ("setFloat32", vec![n(4.0), n(1.5)]),
    ] {
        call(&mut realm, &view, method, &args);
    }
    let bytes = new(&mut realm, "Uint8Array", std::slice::from_ref(&buffer)).unwrap();
    assert_eq!(
        call(&mut realm, &bytes, "join", &[]),
        "18,52,52,18,63,192,0,0"
    );
    let reads = [
        ("getUint16", vec![n(0.0)], "4660"),
        ("getUint16", vec![n(0.0), Value::Boolean(true)], "13330"),
        ("getFloat32", vec![n(4.0)], "1.5"),
        ("getInt8", vec![n(0.0)], "18"),
        ("getInt32", vec![n(0.0)], "305411090"),
        (
            "getUint32",
            vec![n(6.0)],
            "throws RangeError: Offset is outside the bounds of the DataView",
        ),
        (
            "getFloat64",
            vec![n(-1.0)],
            "throws RangeError: Offset is outside the bounds of the DataView",
        ),
    ];
    for (method, args, expected) in reads {
        assert_eq!(call(&mut realm, &view, method, &args), expected, "{method}");
    }
    for (name, expected) in [("byteLength", "8"), ("byteOffset", "0")] {
        let value = ops::get(&mut realm, &view, &Key::name(name)).unwrap();
        assert_eq!(text(&mut realm, Ok(value)), expected);
    }
    let own_buffer = ops::get(&mut realm, &view, &Key::name("buffer")).unwrap();
    assert_eq!(own_buffer.as_object(), buffer.as_object());
    let errors = [
        (
            vec![Value::Object(realm.object())],
            "throws TypeError: First argument to DataView constructor must be an ArrayBuffer",
        ),
        (
            vec![buffer.clone(), n(9.0)],
            "throws RangeError: Start offset 9 is outside the bounds of the buffer",
        ),
        (
            vec![buffer.clone(), n(4.0), n(5.0)],
            "throws RangeError: Invalid DataView length 5",
        ),
    ];
    for (args, expected) in errors {
        let result = new(&mut realm, "DataView", &args);
        assert_eq!(text(&mut realm, result), expected);
    }
    let window = new(&mut realm, "DataView", &[buffer.clone(), n(2.0), n(2.0)]).unwrap();
    let negative = new(&mut realm, "DataView", &[buffer.clone(), n(-1.0)]);
    assert!(text(&mut realm, negative).starts_with("throws RangeError"));
    assert_eq!(
        call(
            &mut realm,
            &window,
            "getUint16",
            &[n(0.0), Value::Boolean(true)]
        ),
        "4660"
    );
    let data_view = global(&mut realm, "DataView");
    let called = ops::call(&mut realm, &data_view, Value::Undefined, &[]);
    assert_eq!(
        text(&mut realm, called),
        "throws TypeError: Constructor DataView requires 'new'"
    );
    let get = ops::get(&mut realm, &view, &Key::name("getInt8")).unwrap();
    let wrong = ops::call(&mut realm, &get, buffer.clone(), &[n(0.0)]);
    assert!(text(&mut realm, wrong).contains("incompatible receiver"));
    // ArrayBuffer statics, accessors and slice with species.
    let is_view =
        |realm: &mut Realm, value: Value| call(realm, &constructor.clone(), "isView", &[value]);
    assert_eq!(is_view(&mut realm, view.clone()), "true");
    assert_eq!(is_view(&mut realm, bytes.clone()), "true");
    assert_eq!(is_view(&mut realm, buffer.clone()), "false");
    for (name, expected) in [
        ("byteLength", "8"),
        ("maxByteLength", "8"),
        ("resizable", "false"),
        ("detached", "false"),
    ] {
        let value = ops::get(&mut realm, &buffer, &Key::name(name)).unwrap();
        assert_eq!(text(&mut realm, Ok(value)), expected, "{name}");
        let getter = realm
            .array_buffer_prototype
            .own_property(&Key::name(name))
            .unwrap();
        let Slot::Accessor {
            get: Some(getter), ..
        } = getter.slot
        else {
            unreachable!()
        };
        let result = ops::call(&mut realm, &Value::Object(getter), n(1.0), &[]);
        assert!(
            text(&mut realm, result).contains("incompatible receiver"),
            "{name}"
        );
    }
    let sliced = call(&mut realm, &buffer, "slice", &[n(4.0)]);
    assert_eq!(sliced, "[object ArrayBuffer]");
    let key = Key::name("constructor");
    let species = Key::Symbol(realm.symbols.species.clone());
    let holder = Value::Object(realm.object());
    ops::set(&mut realm, &buffer, &key, holder.clone()).unwrap();
    assert_eq!(
        call(&mut realm, &buffer, "slice", &[]),
        "[object ArrayBuffer]"
    );
    let same = buffer.clone();
    let returns_same = function(&realm, true, move |_, _, _| Ok(same.clone()));
    let tiny = function(&realm, true, |realm, _, _| {
        let constructor = global(realm, "ArrayBuffer");
        ops::construct(realm, &constructor, &[n(1.0)])
    });
    let not_buffer = function(&realm, true, |realm, _, _| {
        Ok(Value::Object(realm.object()))
    });
    for (species_value, expected) in [
        (
            returns_same,
            "throws TypeError: ArrayBuffer subclass returned this from species constructor",
        ),
        (
            tiny,
            "throws TypeError: species constructor returned an ArrayBuffer that is too small",
        ),
        (
            not_buffer,
            "throws TypeError: species constructor did not return an ArrayBuffer",
        ),
    ] {
        ops::set(&mut realm, &holder, &species, species_value).unwrap();
        assert_eq!(call(&mut realm, &buffer, "slice", &[]), expected);
    }
    ops::set(&mut realm, &buffer, &key, Value::Undefined).unwrap();
    let copy = {
        let function = ops::get(&mut realm, &buffer, &Key::name("slice")).unwrap();
        ops::call(&mut realm, &function, buffer.clone(), &[n(0.0), n(2.0)]).unwrap()
    };
    let copy_bytes = new(&mut realm, "Uint8Array", &[copy]).unwrap();
    assert_eq!(call(&mut realm, &copy_bytes, "join", &[]), "18,52");
    let debug = format!(
        "{:?} {:?}",
        buffer.as_object().unwrap().data().kind,
        view.as_object().unwrap().data().kind
    );
    assert_eq!(debug, "ArrayBuffer(8) DataView(8)");
}

fn big(text: &str) -> Value {
    let integer = crate::bigint::BigInt::parse(&text.encode_utf16().collect::<Vec<_>>()).unwrap();
    Value::BigInt(Rc::new(integer))
}

fn big_array(realm: &mut Realm, name: &str, values: &[&str]) -> Value {
    let source = Value::Object(realm.array(values.iter().map(|value| big(value)).collect()));
    new(realm, name, &[source]).unwrap()
}

#[test]
fn bigint_elements_preserve_exact_words_wrap_and_share_buffers() {
    let mut realm = Realm::new();
    let signed = big_array(
        &mut realm,
        "BigInt64Array",
        &[
            "9007199254740993",
            "18446744073709551615",
            "18446744073709551617",
        ],
    );
    assert_eq!(
        call(&mut realm, &signed, "join", &[]),
        "9007199254740993,-1,1"
    );
    let buffer = ops::get(&mut realm, &signed, &Key::name("buffer")).unwrap();
    let unsigned = new(&mut realm, "BigUint64Array", core::slice::from_ref(&buffer)).unwrap();
    assert_eq!(
        call(&mut realm, &unsigned, "join", &[]),
        "9007199254740993,18446744073709551615,1"
    );
    let copied = new(&mut realm, "BigUint64Array", core::slice::from_ref(&signed)).unwrap();
    assert_eq!(
        call(&mut realm, &copied, "join", &[]),
        "9007199254740993,18446744073709551615,1"
    );
    ops::set(
        &mut realm,
        &unsigned,
        &Key::Index(0),
        big("-18446744073709551617"),
    )
    .unwrap();
    assert_eq!(call(&mut realm, &signed, "join", &[]), "-1,-1,1");
    let dv = new(&mut realm, "DataView", &[buffer]).unwrap();
    let getter = ops::get(&mut realm, &dv, &Key::name("getBigInt64")).unwrap();
    let wrong = Value::Object(realm.object());
    let result = ops::call(&mut realm, &getter, wrong, &[]);
    assert_eq!(
        text(&mut realm, result),
        "throws TypeError: Method DataView.prototype.getBigInt64 called on incompatible receiver #<Object>"
    );

    for little in [false, true] {
        assert_eq!(
            call(
                &mut realm,
                &dv,
                "setBigInt64",
                &[n(1.0), big("-9007199254740993"), Value::Boolean(little)]
            ),
            "undefined"
        );
        assert_eq!(
            call(
                &mut realm,
                &dv,
                "getBigInt64",
                &[n(1.0), Value::Boolean(little)]
            ),
            "-9007199254740993"
        );
        assert_eq!(
            call(
                &mut realm,
                &dv,
                "getBigUint64",
                &[n(1.0), Value::Boolean(little)]
            ),
            "18437736874454810623"
        );
        assert_eq!(
            call(
                &mut realm,
                &dv,
                "setBigUint64",
                &[n(3.0), big("18446744073709551617"), Value::Boolean(little)]
            ),
            "undefined"
        );
        assert_eq!(
            call(
                &mut realm,
                &dv,
                "getBigUint64",
                &[n(3.0), Value::Boolean(little)]
            ),
            "1"
        );
    }
    assert_eq!(
        call(&mut realm, &dv, "setBigInt64", &[n(99.0), n(1.0)]),
        "throws TypeError: Cannot convert 1 to a BigInt"
    );
    assert_eq!(
        call(&mut realm, &dv, "setBigInt64", &[n(99.0), big("1")]),
        "throws RangeError: Offset is outside the bounds of the DataView"
    );
    assert_eq!(
        call(&mut realm, &dv, "getBigUint64", &[n(99.0)]),
        "throws RangeError: Offset is outside the bounds of the DataView"
    );
}

#[test]
fn bigint_typed_arrays_reject_mixed_content_and_species() {
    let mut realm = Realm::new();
    let numbers = typed(&mut realm, "Uint8Array", &[]);
    let integers = big_array(&mut realm, "BigInt64Array", &[]);
    for (name, source) in [
        ("BigInt64Array", numbers.clone()),
        ("Uint8Array", integers.clone()),
    ] {
        let result = new(&mut realm, name, &[source]);
        assert_eq!(
            text(&mut realm, result),
            "throws TypeError: Cannot mix BigInt and other types, use explicit conversions"
        );
    }
    assert_eq!(
        call(&mut realm, &integers, "set", &[numbers]),
        "throws TypeError: Cannot mix BigInt and other types, use explicit conversions"
    );
    let too_large = typed(&mut realm, "Uint8Array", &[1.0]);
    assert_eq!(
        call(&mut realm, &integers, "set", &[too_large]),
        "throws RangeError: offset is out of bounds"
    );
    let source = big_array(&mut realm, "BigInt64Array", &["-1", "2"]);
    let holder = Value::Object(realm.object());
    let species = Key::Symbol(realm.symbols.species.clone());
    let opposite = global(&mut realm, "BigUint64Array");
    ops::set(&mut realm, &holder, &species, opposite).unwrap();
    ops::set(
        &mut realm,
        &source,
        &Key::name("constructor"),
        holder.clone(),
    )
    .unwrap();
    assert_eq!(
        call(&mut realm, &source, "slice", &[]),
        "18446744073709551615,2"
    );
    let numeric = global(&mut realm, "Uint8Array");
    ops::set(&mut realm, &holder, &species, numeric).unwrap();
    assert_eq!(
        call(&mut realm, &source, "slice", &[]),
        "throws TypeError: Cannot mix BigInt and other types, use explicit conversions"
    );
    assert_eq!(call(&mut realm, &source, "subarray", &[]), "255,255");
    assert_eq!(call(&mut realm, &source, "slice", &[n(0.0), n(0.0)]), "");
    let number = function(&realm, false, |_, _, _| Ok(n(3.0)));
    assert_eq!(call(&mut realm, &source, "map", &[number]), "3,3");
    let keep = function(&realm, false, |_, _, _| Ok(Value::Boolean(true)));
    assert_eq!(
        call(&mut realm, &source, "filter", &[keep]),
        "throws TypeError: Cannot convert a BigInt value to a number"
    );
    assert_eq!(
        call(&mut realm, &source, "fill", &[n(1.0), n(0.0), n(0.0)]),
        "throws TypeError: Cannot convert 1 to a BigInt"
    );
    assert_eq!(
        call(&mut realm, &source, "with", &[n(99.0), n(1.0)]),
        "throws TypeError: Cannot convert 1 to a BigInt"
    );
}

#[test]
fn bigint_element_definitions_convert_with_realm_and_validate_before_conversion() {
    let mut realm = Realm::new();
    let made = big_array(&mut realm, "BigInt64Array", &["0"]);
    let object = made.as_object().unwrap().clone();
    for (value, expected) in [
        (Value::Boolean(true), "1"),
        (s("-2"), "-2"),
        (big("9007199254740993"), "9007199254740993"),
    ] {
        assert!(object.define(Key::Index(0), Property::data(value)));
        assert_eq!(call(&mut realm, &made, "join", &[]), expected);
    }
    for value in [s("bad"), n(1.0)] {
        assert!(!object.define(Key::Index(0), Property::data(value)));
    }
    let events = Rc::new(RefCell::new(Vec::new()));
    let capture = events.clone();
    let value = realm.object();
    let value_of = realm.function("valueOf", 0, false, move |_, _, _| {
        capture.borrow_mut().push("number");
        Ok(big("9007199254740993"))
    });
    value.define(
        Key::name("valueOf"),
        Property::data(Value::Object(value_of)),
    );
    let descriptor = crate::object::Descriptor {
        value: Some(Value::Object(value.clone())),
        ..Default::default()
    };
    assert!(
        ops::try_define_property(&mut realm, &object, &Key::Index(0), descriptor.clone()).unwrap()
    );
    assert_eq!(*events.borrow(), vec!["number"]);
    assert_eq!(call(&mut realm, &made, "join", &[]), "9007199254740993");
    assert!(
        !ops::try_define_property(&mut realm, &object, &Key::Index(9), descriptor.clone()).unwrap()
    );
    let forbidden = crate::object::Descriptor {
        configurable: Some(false),
        ..descriptor
    };
    assert!(!ops::try_define_property(&mut realm, &object, &Key::Index(0), forbidden).unwrap());
    assert_eq!(*events.borrow(), vec!["number"]);
    ops::set(&mut realm, &made, &Key::Index(9), Value::Object(value)).unwrap();
    assert_eq!(*events.borrow(), vec!["number", "number"]);
    let invalid = crate::object::Descriptor {
        value: Some(s("bad")),
        ..Default::default()
    };
    let err = ops::try_define_property(&mut realm, &object, &Key::Index(0), invalid).unwrap_err();
    assert!(text(&mut realm, Err(err)).contains("SyntaxError"));
    let inspect = crate::inspect::inspect(&mut realm, &made, 2).unwrap();
    assert_eq!(inspect, "BigInt64Array(1) [ 9007199254740993n ]");
}

#[test]
fn bigint_methods_keep_content_and_numeric_sort_order() {
    let mut realm = Realm::new();
    let made = big_array(
        &mut realm,
        "BigInt64Array",
        &["9007199254740993", "-1", "3", "2"],
    );
    assert_eq!(
        call(&mut realm, &made, "sort", &[]),
        "-1,2,3,9007199254740993"
    );
    let descending = function(&realm, false, |_, _, args| {
        let (Value::BigInt(a), Value::BigInt(b)) = (&args[0], &args[1]) else {
            panic!("BigInt callback arguments")
        };
        Ok(n(match a.compare(b) {
            core::cmp::Ordering::Less => 1.0,
            core::cmp::Ordering::Equal => 0.0,
            core::cmp::Ordering::Greater => -1.0,
        }))
    });
    assert_eq!(
        call(&mut realm, &made, "toSorted", &[descending]),
        "9007199254740993,3,2,-1"
    );
    assert_eq!(
        call(&mut realm, &made, "toReversed", &[]),
        "9007199254740993,3,2,-1"
    );
    assert_eq!(
        call(&mut realm, &made, "with", &[n(-1.0), big("9")]),
        "-1,2,3,9"
    );
    let increment = function(&realm, false, |_, _, args| {
        let Value::BigInt(value) = &args[0] else {
            panic!("BigInt callback argument")
        };
        Ok(Value::BigInt(Rc::new(
            value.add(&crate::bigint::BigInt::from_i64(1)).unwrap(),
        )))
    });
    assert_eq!(
        call(&mut realm, &made, "map", &[increment]),
        "0,3,4,9007199254740994"
    );
    let keep = function(&realm, false, |_, _, args| {
        Ok(Value::Boolean(!ops::strict_equals(&args[0], &big("-1"))))
    });
    assert_eq!(
        call(&mut realm, &made, "filter", &[keep]),
        "2,3,9007199254740993"
    );
    assert_eq!(
        call(&mut realm, &made, "fill", &[s("7"), n(1.0), n(3.0)]),
        "-1,7,7,9007199254740993"
    );
    assert_eq!(
        call(&mut realm, &made, "copyWithin", &[n(1.0), n(0.0), n(2.0)]),
        "-1,-1,7,9007199254740993"
    );
    assert_eq!(
        call(&mut realm, &made, "reverse", &[]),
        "9007199254740993,7,-1,-1"
    );
    let ctor = global(&mut realm, "BigUint64Array");
    assert_eq!(
        call(
            &mut realm,
            &ctor,
            "of",
            &[Value::Boolean(true), s("42"), big("-1")]
        ),
        "1,42,18446744073709551615"
    );
}

#[test]
fn array_like_bigint_reads_and_conversions_run_in_index_order() {
    let mut realm = Realm::new();
    let events = Rc::new(RefCell::new(Vec::new()));
    let source = realm.object();
    source.define(Key::name("length"), Property::data(n(2.0)));
    for index in 0..2 {
        let log = events.clone();
        let getter = realm.function("get", 0, false, move |realm, _, _| {
            log.borrow_mut().push(format!("get{index}"));
            let value = realm.object();
            let log = log.clone();
            let converter = realm.function("valueOf", 0, false, move |_, _, _| {
                log.borrow_mut().push(format!("convert{index}"));
                Ok(big("1"))
            });
            value.define(
                Key::name("valueOf"),
                Property::data(Value::Object(converter)),
            );
            Ok(Value::Object(value))
        });
        source.define(
            Key::Index(index),
            Property {
                slot: Slot::Accessor {
                    get: Some(getter),
                    set: None,
                },
                enumerable: true,
                configurable: true,
            },
        );
    }
    let source = Value::Object(source);
    let made = new(&mut realm, "BigInt64Array", core::slice::from_ref(&source)).unwrap();
    assert_eq!(call(&mut realm, &made, "join", &[]), "1,1");
    assert_eq!(*events.borrow(), ["get0", "convert0", "get1", "convert1"]);
    events.borrow_mut().clear();
    assert_eq!(
        call(&mut realm, &made, "set", core::slice::from_ref(&source)),
        "undefined"
    );
    assert_eq!(*events.borrow(), ["get0", "convert0", "get1", "convert1"]);
    events.borrow_mut().clear();
    let constructor = global(&mut realm, "BigInt64Array");
    assert_eq!(call(&mut realm, &constructor, "from", &[source]), "1,1");
    assert_eq!(*events.borrow(), ["get0", "convert0", "get1", "convert1"]);
}
