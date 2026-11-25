use super::*;
use alloc::rc::Rc;
use core::cell::RefCell;

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn array(realm: &Realm, values: &[Value]) -> Value {
    Value::Object(realm.array(values.to_vec()))
}

/// An array with holes where `None` appears.
fn holey(realm: &Realm, values: &[Option<Value>]) -> Value {
    Value::Object(realm.sparse_array(values.to_vec()))
}

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

fn get(realm: &mut Realm, target: &Value, name: &str) -> Value {
    ops::get(realm, target, &Key::name(name)).unwrap()
}

/// Calls `Array.prototype[name]` on a receiver.
fn call(realm: &mut Realm, this: &Value, name: &str, args: &[Value]) -> Completion {
    let prototype = Value::Object(realm.array_prototype.clone());
    let method = ops::get(realm, &prototype, &Key::name(name)).unwrap();
    ops::call(realm, &method, this.clone(), args)
}

/// Calls a static function of `Array`.
fn statics(realm: &mut Realm, this: &Value, name: &str, args: &[Value]) -> Completion {
    let constructor = global(realm, "Array");
    let function = get(realm, &constructor, name);
    ops::call(realm, &function, this.clone(), args)
}

/// Elements up to `length`, with holes shown as `_`.
fn show(realm: &mut Realm, value: &Value) -> String {
    let length = length_of(realm, value).unwrap();
    let object = value.as_object().unwrap().clone();
    (0..length)
        .map(|index| match object.own_property(&index_key(index)) {
            None => "_".to_owned(),
            Some(_) => {
                let element = ops::get(realm, value, &index_key(index)).unwrap();
                match element {
                    Value::Undefined => "undefined".to_owned(),
                    element => ops::to_string(realm, &element).unwrap().to_rust_lossy(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

fn function(
    realm: &Realm,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Value {
    Value::Object(realm.function("f", 0, false, code))
}

fn equal(actual: &Value, expected: &Value) {
    assert!(
        ops::strict_equals(actual, expected),
        "{actual:?} != {expected:?}"
    );
}

/// An ordinary object with a `length` and indexed elements.
fn array_like(realm: &Realm, length: f64, values: &[(u32, Value)]) -> Value {
    let object = realm.object();
    object.define(Key::name("length"), Property::data(n(length)));
    for (index, value) in values {
        object.define(Key::Index(*index), Property::data(value.clone()));
    }
    Value::Object(object)
}

#[test]
fn numeric_conversions_and_keys_follow_the_specification() {
    let mut realm = Realm::new();
    for (value, integer) in [
        (n(f64::NAN), 0.0),
        (n(f64::INFINITY), f64::INFINITY),
        (n(-2.7), -2.0),
        (n(-0.5), 0.0),
        (s("3.9"), 3.0),
    ] {
        let result = to_integer_or_infinity(&mut realm, &value).unwrap();
        assert_eq!(result, integer);
        assert!(!result.is_sign_negative() || result != 0.0);
    }
    assert_eq!(to_length(&mut realm, &n(-5.0)).unwrap(), 0);
    assert_eq!(
        to_length(&mut realm, &n(f64::INFINITY)).unwrap(),
        MAX_LENGTH
    );
    assert_eq!(index_key(7), Key::Index(7));
    assert_eq!(index_key(u64::from(u32::MAX)), Key::name("4294967295"));
    assert_eq!(
        relative(&mut realm, &n(f64::NEG_INFINITY), 5, 9).unwrap(),
        0
    );
    assert_eq!(relative(&mut realm, &n(-2.0), 5, 9).unwrap(), 3);
    assert_eq!(relative(&mut realm, &n(7.0), 5, 9).unwrap(), 5);
    assert_eq!(relative(&mut realm, &Value::Undefined, 5, 9).unwrap(), 9);
    assert!(!is_callable(&n(1.0)));
    assert!(!is_constructor(&function(&realm, |_, _, _| Ok(
        Value::Undefined
    ))));
}

#[test]
fn the_constructor_creates_arrays_from_lengths_and_elements() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "Array");
    let empty = ops::call(&mut realm, &constructor, Value::Undefined, &[]).unwrap();
    assert_eq!(show(&mut realm, &empty), "");
    let sized = ops::construct(&mut realm, &constructor, &[n(3.0)]).unwrap();
    assert_eq!(show(&mut realm, &sized), "_,_,_");
    let listed = ops::construct(&mut realm, &constructor, &[n(1.0), s("b")]).unwrap();
    assert_eq!(show(&mut realm, &listed), "1,b");
    let single = ops::call(&mut realm, &constructor, Value::Undefined, &[s("3")]).unwrap();
    assert_eq!(show(&mut realm, &single), "3");
    for length in [-1.0, 1.5, 4_294_967_296.0] {
        let invalid = ops::construct(&mut realm, &constructor, &[n(length)]);
        assert_eq!(
            message(&mut realm, invalid),
            "RangeError: Invalid array length"
        );
    }
    // Subclass construction takes the new target's prototype.
    let target = realm.function("Target", 0, true, |_, _, _| Ok(Value::Undefined));
    let custom = realm.object();
    target.define(
        Key::name("prototype"),
        Property::data(Value::Object(custom.clone())),
    );
    let instance = ops::construct_target(
        &mut realm,
        &constructor,
        &[n(2.0)],
        &Value::Object(target.clone()),
    )
    .unwrap();
    assert_eq!(instance.as_object().unwrap().data().prototype, Some(custom));
    assert!(instance.as_object().unwrap().is_array());
    target.define(Key::name("prototype"), Property::data(n(1.0)));
    let fallback =
        ops::construct_target(&mut realm, &constructor, &[], &Value::Object(target)).unwrap();
    assert_eq!(
        fallback.as_object().unwrap().data().prototype,
        Some(realm.array_prototype.clone())
    );
    let species = Key::Symbol(realm.symbols.species.clone());
    equal(
        &ops::get(&mut realm, &constructor, &species).unwrap(),
        &constructor,
    );
    let array = array(&realm, &[]);
    let is_array = statics(&mut realm, &constructor, "isArray", &[array]).unwrap();
    equal(&is_array, &Value::Boolean(true));
    let not_array = statics(&mut realm, &constructor, "isArray", &[n(1.0)]).unwrap();
    equal(&not_array, &Value::Boolean(false));
    let prototype = Value::Object(realm.array_prototype.clone());
    let unscopables = Key::Symbol(realm.symbols.unscopables.clone());
    let list = ops::get(&mut realm, &prototype, &unscopables).unwrap();
    equal(&get(&mut realm, &list, "flat"), &Value::Boolean(true));
}

/// An iterable whose iterator yields `values` and records `return` calls.
fn iterable(realm: &Realm, values: Vec<Value>, closed: Rc<RefCell<u32>>) -> Value {
    let iterable = realm.object();
    let state = Rc::new(RefCell::new(0usize));
    let next = function(realm, move |realm, _, _| {
        let index = *state.borrow();
        *state.borrow_mut() += 1;
        let result = realm.object();
        result.define(
            Key::name("done"),
            Property::data(Value::Boolean(index >= values.len())),
        );
        result.define(
            Key::name("value"),
            Property::data(values.get(index).cloned().unwrap_or_default()),
        );
        Ok(Value::Object(result))
    });
    let close = function(realm, move |realm, _, _| {
        *closed.borrow_mut() += 1;
        Ok(Value::Object(realm.object()))
    });
    let iterator = realm.object();
    iterator.define(Key::name("next"), Property::data(next));
    iterator.define(Key::name("return"), Property::data(close));
    let iterator = Value::Object(iterator);
    let method = function(realm, move |_, _, _| Ok(iterator.clone()));
    iterable.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::data(method),
    );
    Value::Object(iterable)
}

#[test]
fn from_and_of_build_arrays_and_subclass_instances() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "Array");
    let like = array_like(&realm, 3.0, &[(0, s("a")), (2, s("c"))]);
    let copied = statics(
        &mut realm,
        &constructor,
        "from",
        core::slice::from_ref(&like),
    )
    .unwrap();
    assert_eq!(show(&mut realm, &copied), "a,undefined,c");
    let double = function(&realm, |realm, _, args| {
        let text = ops::to_string(realm, &args[0])?;
        let index = ops::to_string(realm, &args[1])?;
        Ok(Value::String(text.concat(&index)))
    });
    let mapped = statics(&mut realm, &constructor, "from", &[s("xy"), double.clone()]).unwrap();
    assert_eq!(show(&mut realm, &mapped), "x0,y1");
    let closed = Rc::new(RefCell::new(0));
    let values = iterable(&realm, vec![n(1.0), n(2.0)], closed.clone());
    let from_iterator = statics(&mut realm, &constructor, "from", &[values]).unwrap();
    assert_eq!(show(&mut realm, &from_iterator), "1,2");
    let fails = statics(&mut realm, &constructor, "from", &[like.clone(), n(1.0)]);
    assert!(message(&mut realm, fails).contains("mapping function"));
    let bad = realm.object();
    bad.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::data(n(1.0)),
    );
    let bad = statics(&mut realm, &constructor, "from", &[Value::Object(bad)]);
    assert!(message(&mut realm, bad).contains("Symbol.iterator is not a function"));
    // An abrupt mapping closes the iterator.
    let thrower = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "map"))
    });
    let values = iterable(&realm, vec![n(1.0)], closed.clone());
    let thrown = statics(&mut realm, &constructor, "from", &[values, thrower.clone()]);
    assert_eq!(message(&mut realm, thrown), "RangeError: map");
    assert_eq!(*closed.borrow(), 1);
    let thrown = statics(&mut realm, &constructor, "from", &[like.clone(), thrower]);
    assert_eq!(message(&mut realm, thrown), "RangeError: map");
    // A constructor receiver creates instances; a frozen one rejects elements.
    let calls = Rc::new(RefCell::new(Vec::new()));
    let seen = calls.clone();
    let custom = Value::Object(realm.function("Custom", 0, true, move |_, _, args| {
        seen.borrow_mut().push(args.len());
        Ok(Value::Undefined)
    }));
    let instance = statics(&mut realm, &custom, "from", core::slice::from_ref(&like)).unwrap();
    assert_eq!(show(&mut realm, &instance), "a,undefined,c");
    let instance = statics(&mut realm, &custom, "of", &[n(7.0), n(8.0)]).unwrap();
    assert_eq!(show(&mut realm, &instance), "7,8");
    let values = iterable(&realm, vec![n(5.0)], closed.clone());
    let instance = statics(&mut realm, &custom, "from", &[values]).unwrap();
    assert_eq!(show(&mut realm, &instance), "5");
    assert_eq!(*calls.borrow(), vec![1, 1, 0]);
    let frozen = Value::Object(realm.function("Frozen", 0, true, |realm, _, _| {
        let object = realm.object();
        object.prevent_extensions();
        Ok(Value::Object(object))
    }));
    let values = iterable(&realm, vec![n(1.0)], closed.clone());
    let rejected = statics(&mut realm, &frozen, "from", &[values]);
    assert!(message(&mut realm, rejected).contains("Cannot define property 0"));
    assert_eq!(*closed.borrow(), 2);
    let of = statics(&mut realm, &Value::Undefined, "of", &[n(1.0)]).unwrap();
    assert_eq!(show(&mut realm, &of), "1");
    let empty = statics(&mut realm, &Value::Undefined, "from", &[like]).unwrap();
    assert!(empty.as_object().unwrap().is_array());
}

#[test]
fn reading_methods_handle_holes_negatives_and_absent_values() {
    let mut realm = Realm::new();
    let values = holey(
        &realm,
        &[Some(n(1.0)), None, Some(n(f64::NAN)), Some(n(1.0))],
    );
    equal(
        &call(&mut realm, &values, "at", &[n(-1.0)]).unwrap(),
        &n(1.0),
    );
    equal(
        &call(&mut realm, &values, "at", &[n(9.0)]).unwrap(),
        &Value::Undefined,
    );
    equal(
        &call(&mut realm, &values, "at", &[n(-9.0)]).unwrap(),
        &Value::Undefined,
    );
    equal(
        &call(&mut realm, &values, "includes", &[n(f64::NAN)]).unwrap(),
        &Value::Boolean(true),
    );
    equal(
        &call(&mut realm, &values, "includes", &[Value::Undefined]).unwrap(),
        &Value::Boolean(true),
    );
    equal(
        &call(&mut realm, &values, "includes", &[n(1.0), n(4.0)]).unwrap(),
        &Value::Boolean(false),
    );
    equal(
        &call(&mut realm, &values, "indexOf", &[n(f64::NAN)]).unwrap(),
        &n(-1.0),
    );
    equal(
        &call(&mut realm, &values, "indexOf", &[n(1.0), n(1.0)]).unwrap(),
        &n(3.0),
    );
    equal(
        &call(&mut realm, &values, "indexOf", &[Value::Undefined]).unwrap(),
        &n(-1.0),
    );
    equal(
        &call(&mut realm, &values, "lastIndexOf", &[n(1.0)]).unwrap(),
        &n(3.0),
    );
    equal(
        &call(&mut realm, &values, "lastIndexOf", &[n(1.0), n(-2.0)]).unwrap(),
        &n(0.0),
    );
    equal(
        &call(&mut realm, &values, "lastIndexOf", &[n(1.0), n(-9.0)]).unwrap(),
        &n(-1.0),
    );
    equal(
        &call(&mut realm, &values, "lastIndexOf", &[n(9.0), n(99.0)]).unwrap(),
        &n(-1.0),
    );
    let empty = array(&realm, &[]);
    for name in ["includes", "indexOf", "lastIndexOf"] {
        let result = call(&mut realm, &empty, name, &[n(1.0)]).unwrap();
        assert!(
            matches!(result, Value::Number(_) | Value::Boolean(false)),
            "{name}"
        );
    }
    let joined = call(&mut realm, &values, "join", &[s("-")]).unwrap();
    equal(&joined, &s("1--NaN-1"));
    let nullish = array(&realm, &[Value::Null, Value::Undefined, n(2.0)]);
    equal(&call(&mut realm, &nullish, "join", &[]).unwrap(), &s(",,2"));
    // Cyclic arrays join as empty strings instead of recursing.
    let cyclic = realm.array(vec![n(1.0)]);
    cyclic.define(Key::Index(1), Property::data(Value::Object(cyclic.clone())));
    equal(
        &call(&mut realm, &Value::Object(cyclic.clone()), "join", &[]).unwrap(),
        &s("1,"),
    );
    equal(
        &call(&mut realm, &Value::Object(cyclic), "toString", &[]).unwrap(),
        &s("1,"),
    );
    let plain = Value::Object(realm.object());
    equal(
        &call(&mut realm, &plain, "toString", &[]).unwrap(),
        &s("[object Object]"),
    );
    let primitive = call(&mut realm, &s("ab"), "join", &[s("+")]).unwrap();
    equal(&primitive, &s("a+b"));
    let localized = realm.object();
    let label = function(&realm, |_, _, _| Ok(s("L")));
    localized.define(Key::name("toLocaleString"), Property::data(label));
    let locale = array(
        &realm,
        &[Value::Object(localized), Value::Null, Value::Undefined],
    );
    equal(
        &call(&mut realm, &locale, "toLocaleString", &[]).unwrap(),
        &s("L,,"),
    );
    let failing = realm.object();
    let throwing = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "length"))
    });
    failing.define(
        Key::name("length"),
        Property {
            slot: Slot::Accessor {
                get: throwing.as_object().cloned(),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let failing = Value::Object(failing);
    for name in [
        "at",
        "join",
        "includes",
        "indexOf",
        "lastIndexOf",
        "toLocaleString",
        "slice",
    ] {
        let thrown = call(&mut realm, &failing, name, &[]);
        assert_eq!(message(&mut realm, thrown), "RangeError: length", "{name}");
    }
    let receiver = call(&mut realm, &Value::Undefined, "at", &[]);
    assert!(message(&mut realm, receiver).starts_with("TypeError"));
}

#[test]
fn iterating_methods_call_back_with_elements_indexes_and_receivers() {
    let mut realm = Realm::new();
    let values = holey(&realm, &[Some(n(1.0)), None, Some(n(3.0))]);
    let log = Rc::new(RefCell::new(Vec::new()));
    let record = log.clone();
    let visit = function(&realm, move |realm, this, args| {
        let this = ops::to_string(realm, &this)?.to_rust_lossy();
        let value = ops::to_string(realm, &args[0])?.to_rust_lossy();
        let index = ops::to_string(realm, &args[1])?.to_rust_lossy();
        record.borrow_mut().push(format!("{this}:{value}@{index}"));
        let odd = ops::to_number(realm, &args[0])? % 2.0 == 1.0;
        Ok(Value::Boolean(odd))
    });
    call(&mut realm, &values, "forEach", &[visit.clone(), s("t")]).unwrap();
    assert_eq!(*log.borrow(), ["t:1@0", "t:3@2"]);
    equal(
        &call(&mut realm, &values, "every", std::slice::from_ref(&visit)).unwrap(),
        &Value::Boolean(true),
    );
    equal(
        &call(&mut realm, &values, "some", std::slice::from_ref(&visit)).unwrap(),
        &Value::Boolean(true),
    );
    let evens = array(&realm, &[n(2.0), n(4.0)]);
    equal(
        &call(&mut realm, &evens, "every", std::slice::from_ref(&visit)).unwrap(),
        &Value::Boolean(false),
    );
    equal(
        &call(&mut realm, &evens, "some", std::slice::from_ref(&visit)).unwrap(),
        &Value::Boolean(false),
    );
    let mapped = call(&mut realm, &values, "map", std::slice::from_ref(&visit)).unwrap();
    assert_eq!(show(&mut realm, &mapped), "true,_,true");
    let mixed = array(&realm, &[n(1.0), n(2.0), n(3.0)]);
    let kept = call(&mut realm, &mixed, "filter", std::slice::from_ref(&visit)).unwrap();
    assert_eq!(show(&mut realm, &kept), "1,3");
    for (name, expected) in [
        ("find", n(1.0)),
        ("findIndex", n(0.0)),
        ("findLast", n(3.0)),
        ("findLastIndex", n(2.0)),
    ] {
        equal(
            &call(&mut realm, &mixed, name, std::slice::from_ref(&visit)).unwrap(),
            &expected,
        );
    }
    for (name, expected) in [
        ("find", Value::Undefined),
        ("findIndex", n(-1.0)),
        ("findLast", Value::Undefined),
        ("findLastIndex", n(-1.0)),
    ] {
        equal(
            &call(&mut realm, &evens, name, std::slice::from_ref(&visit)).unwrap(),
            &expected,
        );
    }
    for name in [
        "every",
        "some",
        "forEach",
        "map",
        "filter",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "reduce",
        "reduceRight",
        "flatMap",
    ] {
        let thrown = call(&mut realm, &mixed, name, &[n(1.0)]);
        assert!(
            message(&mut realm, thrown).contains("requires a function"),
            "{name}"
        );
    }
    let sum = function(&realm, |realm, _, args| ops::add(realm, &args[0], &args[1]));
    let words = holey(&realm, &[Some(s("a")), None, Some(s("b")), Some(s("c"))]);
    equal(
        &call(&mut realm, &words, "reduce", std::slice::from_ref(&sum)).unwrap(),
        &s("abc"),
    );
    equal(
        &call(
            &mut realm,
            &words,
            "reduceRight",
            std::slice::from_ref(&sum),
        )
        .unwrap(),
        &s("cba"),
    );
    equal(
        &call(&mut realm, &words, "reduce", &[sum.clone(), s(">")]).unwrap(),
        &s(">abc"),
    );
    let gaps = holey(&realm, &[None, None]);
    let thrown = call(&mut realm, &gaps, "reduce", std::slice::from_ref(&sum));
    assert!(message(&mut realm, thrown).contains("reduce of empty array with no initial value"));
    let thrown = call(&mut realm, &gaps, "reduceRight", &[sum]);
    assert!(message(&mut realm, thrown).contains("reduceRight of empty array"));
}

#[test]
fn species_constructors_decide_the_results_of_derived_arrays() {
    let mut realm = Realm::new();
    let identity = function(&realm, |_, _, args| Ok(args[0].clone()));
    // Array-like receivers create plain arrays.
    let like = array_like(&realm, 2.0, &[(0, n(1.0)), (1, n(2.0))]);
    let mapped = call(&mut realm, &like, "map", std::slice::from_ref(&identity)).unwrap();
    assert!(mapped.as_object().unwrap().is_array());
    let values = realm.array(vec![n(1.0), n(2.0)]);
    let receiver = Value::Object(values.clone());
    let species = Key::Symbol(realm.symbols.species.clone());
    for constructor in [Value::Undefined, Value::Object(realm.object())] {
        values.define(Key::name("constructor"), Property::data(constructor));
        let result = call(
            &mut realm,
            &receiver,
            "map",
            std::slice::from_ref(&identity),
        )
        .unwrap();
        assert!(result.as_object().unwrap().is_array());
    }
    let holder = realm.object();
    holder.define(species.clone(), Property::data(Value::Null));
    values.define(
        Key::name("constructor"),
        Property::data(Value::Object(holder.clone())),
    );
    let result = call(&mut realm, &receiver, "slice", &[]).unwrap();
    assert_eq!(show(&mut realm, &result), "1,2");
    holder.define(species.clone(), Property::data(n(1.0)));
    let thrown = call(
        &mut realm,
        &receiver,
        "filter",
        std::slice::from_ref(&identity),
    );
    assert!(message(&mut realm, thrown).contains("species constructor is not a constructor"));
    let lengths = Rc::new(RefCell::new(Vec::new()));
    let seen = lengths.clone();
    let custom = realm.function("Custom", 0, true, move |realm, _, args| {
        seen.borrow_mut().push(ops::to_number(realm, &args[0])?);
        Ok(Value::Undefined)
    });
    holder.define(species.clone(), Property::data(Value::Object(custom)));
    for (name, args) in [
        ("map", vec![identity.clone()]),
        ("filter", vec![identity.clone()]),
        ("slice", vec![n(1.0)]),
        ("splice", vec![n(0.0), n(1.0)]),
        ("concat", vec![]),
        ("flat", vec![]),
        ("flatMap", vec![identity.clone()]),
    ] {
        let result = call(&mut realm, &receiver, name, &args).unwrap();
        assert!(!result.as_object().unwrap().is_array(), "{name}");
    }
    assert_eq!(*lengths.borrow(), vec![2.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0]);
    let frozen = realm.function("Frozen", 0, true, |realm, _, _| {
        let object = realm.object();
        object.prevent_extensions();
        Ok(Value::Object(object))
    });
    holder.define(species, Property::data(Value::Object(frozen)));
    let thrown = call(&mut realm, &receiver, "map", &[identity]);
    assert!(message(&mut realm, thrown).contains("Cannot define property 0"));
}

#[test]
fn copying_methods_flatten_concatenate_slice_and_copy_within() {
    let mut realm = Realm::new();
    let first = holey(&realm, &[Some(n(1.0)), None]);
    let second = array(&realm, &[n(3.0)]);
    let spread = array_like(&realm, 2.0, &[(0, s("x")), (1, s("y"))]);
    let flag = Key::Symbol(realm.symbols.is_concat_spreadable.clone());
    spread
        .as_object()
        .unwrap()
        .define(flag.clone(), Property::data(Value::Boolean(true)));
    let kept = array(&realm, &[n(9.0)]);
    kept.as_object()
        .unwrap()
        .define(flag.clone(), Property::data(Value::Boolean(false)));
    let joined = call(
        &mut realm,
        &first,
        "concat",
        &[second, s("z"), spread, kept],
    )
    .unwrap();
    assert_eq!(show(&mut realm, &joined), "1,_,3,z,x,y,9");
    let huge = array_like(&realm, MAX_LENGTH as f64, &[]);
    huge.as_object()
        .unwrap()
        .define(flag, Property::data(Value::Boolean(true)));
    let thrown = call(&mut realm, &first, "concat", std::slice::from_ref(&huge));
    assert!(message(&mut realm, thrown).contains("exceeds the maximum length"));

    let nested = holey(
        &realm,
        &[
            Some(n(1.0)),
            None,
            Some(array(&realm, &[n(2.0), array(&realm, &[n(3.0)])])),
        ],
    );
    let flat = call(&mut realm, &nested, "flat", &[]).unwrap();
    assert_eq!(show(&mut realm, &flat), "1,2,3");
    let deep = call(&mut realm, &nested, "flat", &[n(f64::INFINITY)]).unwrap();
    assert_eq!(show(&mut realm, &deep), "1,2,3");
    let shallow = call(&mut realm, &nested, "flat", &[n(-1.0)]).unwrap();
    assert_eq!(length_of(&mut realm, &shallow).unwrap(), 2);
    let pair = function(&realm, |realm, _, args| {
        Ok(array(realm, &[args[0].clone(), args[1].clone()]))
    });
    let letters = array(&realm, &[s("a"), s("b")]);
    let flat_mapped = call(&mut realm, &letters, "flatMap", &[pair]).unwrap();
    assert_eq!(show(&mut realm, &flat_mapped), "a,0,b,1");

    let values = array(&realm, &[n(0.0), n(1.0), n(2.0), n(3.0), n(4.0)]);
    let part = call(&mut realm, &values, "slice", &[n(-3.0), n(-1.0)]).unwrap();
    assert_eq!(show(&mut realm, &part), "2,3");
    let empty = call(&mut realm, &values, "slice", &[n(3.0), n(1.0)]).unwrap();
    assert_eq!(show(&mut realm, &empty), "");
    let gap_first = holey(&realm, &[None, Some(n(1.0))]);
    let sparse = call(&mut realm, &gap_first, "slice", &[]).unwrap();
    assert_eq!(show(&mut realm, &sparse), "_,1");
    let huge = array_like(&realm, 5e9, &[]);
    let thrown = call(&mut realm, &huge, "slice", &[]);
    assert_eq!(
        message(&mut realm, thrown),
        "RangeError: Invalid array length"
    );

    let forward = array(&realm, &[n(0.0), n(1.0), n(2.0), n(3.0), n(4.0)]);
    call(
        &mut realm,
        &forward,
        "copyWithin",
        &[n(1.0), n(0.0), n(3.0)],
    )
    .unwrap();
    assert_eq!(show(&mut realm, &forward), "0,0,1,2,4");
    call(&mut realm, &forward, "copyWithin", &[n(0.0), n(2.0)]).unwrap();
    assert_eq!(show(&mut realm, &forward), "1,2,4,2,4");
    let gaps = holey(&realm, &[None, Some(n(1.0))]);
    call(&mut realm, &gaps, "copyWithin", &[n(1.0), n(0.0)]).unwrap();
    assert_eq!(show(&mut realm, &gaps), "_,_");

    let filled = array(&realm, &[n(0.0), n(0.0), n(0.0)]);
    call(&mut realm, &filled, "fill", &[n(7.0), n(1.0)]).unwrap();
    assert_eq!(show(&mut realm, &filled), "0,7,7");
    call(&mut realm, &filled, "fill", &[n(5.0), n(-3.0), n(-2.0)]).unwrap();
    assert_eq!(show(&mut realm, &filled), "5,7,7");
}

#[test]
fn mutating_methods_push_pop_shift_unshift_reverse_and_splice() {
    let mut realm = Realm::new();
    let values = array(&realm, &[n(1.0)]);
    equal(
        &call(&mut realm, &values, "push", &[n(2.0), n(3.0)]).unwrap(),
        &n(3.0),
    );
    equal(&call(&mut realm, &values, "pop", &[]).unwrap(), &n(3.0));
    equal(
        &call(&mut realm, &values, "unshift", &[n(0.0)]).unwrap(),
        &n(3.0),
    );
    equal(&call(&mut realm, &values, "unshift", &[]).unwrap(), &n(3.0));
    assert_eq!(show(&mut realm, &values), "0,1,2");
    equal(&call(&mut realm, &values, "shift", &[]).unwrap(), &n(0.0));
    assert_eq!(show(&mut realm, &values), "1,2");
    let empty = array(&realm, &[]);
    equal(
        &call(&mut realm, &empty, "pop", &[]).unwrap(),
        &Value::Undefined,
    );
    equal(
        &call(&mut realm, &empty, "shift", &[]).unwrap(),
        &Value::Undefined,
    );
    let like = array_like(&realm, 1.0, &[(0, s("a"))]);
    equal(
        &call(&mut realm, &like, "push", &[s("b")]).unwrap(),
        &n(2.0),
    );
    equal(&get(&mut realm, &like, "length"), &n(2.0));
    let gappy = holey(&realm, &[Some(n(1.0)), None, Some(n(3.0))]);
    call(&mut realm, &gappy, "shift", &[]).unwrap();
    assert_eq!(show(&mut realm, &gappy), "_,3");
    call(&mut realm, &gappy, "unshift", &[n(9.0)]).unwrap();
    assert_eq!(show(&mut realm, &gappy), "9,_,3");
    let full = array_like(&realm, MAX_LENGTH as f64, &[]);
    for name in ["push", "unshift"] {
        let thrown = call(&mut realm, &full, name, &[n(1.0)]);
        assert!(
            message(&mut realm, thrown).contains("exceeds the maximum length"),
            "{name}"
        );
    }
    // A non-configurable element cannot be deleted.
    let fixed = realm.array(vec![n(1.0)]);
    fixed.define(
        Key::Index(0),
        Property {
            slot: Slot::Data {
                value: n(1.0),
                writable: true,
            },
            enumerable: true,
            configurable: false,
        },
    );
    let thrown = call(&mut realm, &Value::Object(fixed), "pop", &[]);
    assert!(message(&mut realm, thrown).contains("Cannot delete property '0'"));

    let reversed = holey(
        &realm,
        &[
            Some(n(1.0)),
            None,
            Some(n(3.0)),
            None,
            Some(n(5.0)),
            Some(n(6.0)),
        ],
    );
    call(&mut realm, &reversed, "reverse", &[]).unwrap();
    assert_eq!(show(&mut realm, &reversed), "6,5,_,3,_,1");
    let both = holey(&realm, &[None, Some(n(2.0)), None]);
    call(&mut realm, &both, "reverse", &[]).unwrap();
    assert_eq!(show(&mut realm, &both), "_,2,_");

    let list = array(&realm, &[n(0.0), n(1.0), n(2.0), n(3.0), n(4.0)]);
    let removed = call(&mut realm, &list, "splice", &[n(1.0), n(2.0), s("a")]).unwrap();
    assert_eq!(show(&mut realm, &removed), "1,2");
    assert_eq!(show(&mut realm, &list), "0,a,3,4");
    call(
        &mut realm,
        &list,
        "splice",
        &[n(-1.0), n(0.0), s("b"), s("c")],
    )
    .unwrap();
    assert_eq!(show(&mut realm, &list), "0,a,3,b,c,4");
    let tail = call(&mut realm, &list, "splice", &[n(4.0)]).unwrap();
    assert_eq!(show(&mut realm, &tail), "c,4");
    let none = call(&mut realm, &list, "splice", &[]).unwrap();
    assert_eq!(show(&mut realm, &none), "");
    let same = call(&mut realm, &list, "splice", &[n(1.0), n(1.0), s("A")]).unwrap();
    assert_eq!(show(&mut realm, &same), "a");
    assert_eq!(show(&mut realm, &list), "0,A,3,b");
    let gaps = holey(&realm, &[None, Some(n(1.0)), None]);
    let taken = call(&mut realm, &gaps, "splice", &[n(0.0), n(2.0)]).unwrap();
    assert_eq!(show(&mut realm, &taken), "_,1");
    let thrown = call(&mut realm, &full, "splice", &[n(0.0), n(0.0), n(1.0)]);
    assert!(message(&mut realm, thrown).contains("exceeds the maximum length"));
}

#[test]
fn sorting_is_stable_puts_undefined_last_and_tolerates_bad_comparators() {
    let mut realm = Realm::new();
    let values = holey(
        &realm,
        &[
            Some(s("b")),
            None,
            Some(Value::Undefined),
            Some(n(10.0)),
            Some(n(9.0)),
            Some(s("a")),
        ],
    );
    call(&mut realm, &values, "sort", &[]).unwrap();
    assert_eq!(show(&mut realm, &values), "10,9,a,b,undefined,_");
    let pairs = array(&realm, &[s("b1"), s("a1"), s("b2"), s("a2"), s("c1")]);
    let by_letter = function(&realm, |realm, _, args| {
        let left = ops::to_string(realm, &args[0])?.units()[0];
        let right = ops::to_string(realm, &args[1])?.units()[0];
        Ok(n(f64::from(left) - f64::from(right)))
    });
    call(&mut realm, &pairs, "sort", std::slice::from_ref(&by_letter)).unwrap();
    assert_eq!(show(&mut realm, &pairs), "a1,a2,b1,b2,c1");
    let numbers: Vec<Value> = (0..40)
        .map(|index| n(f64::from((index * 7) % 40)))
        .collect();
    let numbers = array(&realm, &numbers);
    let random = function(&realm, |_, _, args| {
        // An inconsistent comparator must not fail the sort.
        Ok(n(
            if matches!(args[0], Value::Number(x) if x as i64 % 3 == 0) {
                1.0
            } else {
                -1.0
            },
        ))
    });
    call(&mut realm, &numbers, "sort", &[random]).unwrap();
    assert_eq!(length_of(&mut realm, &numbers).unwrap(), 40);
    let nan = function(&realm, |_, _, _| Ok(n(f64::NAN)));
    let keep = array(&realm, &[n(2.0), n(1.0)]);
    call(&mut realm, &keep, "sort", &[nan]).unwrap();
    assert_eq!(show(&mut realm, &keep), "2,1");
    let thrown = call(&mut realm, &keep, "sort", &[n(1.0)]);
    assert!(message(&mut realm, thrown).contains("must be a function or undefined"));
    let thrower = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "compare"))
    });
    let thrown = call(&mut realm, &keep, "sort", &[thrower]);
    assert_eq!(message(&mut realm, thrown), "RangeError: compare");
    let sorted = call(&mut realm, &pairs, "toSorted", &[]).unwrap();
    assert_eq!(show(&mut realm, &sorted), "a1,a2,b1,b2,c1");
    let thrown = call(&mut realm, &pairs, "toSorted", &[s("x")]);
    assert!(message(&mut realm, thrown).contains("toSorted must be a function"));
}

#[test]
fn copying_variants_leave_the_receiver_unchanged() {
    let mut realm = Realm::new();
    let values = holey(&realm, &[Some(n(1.0)), None, Some(n(3.0))]);
    let reversed = call(&mut realm, &values, "toReversed", &[]).unwrap();
    assert_eq!(show(&mut realm, &reversed), "3,undefined,1");
    let spliced = call(
        &mut realm,
        &values,
        "toSpliced",
        &[n(1.0), n(1.0), s("x"), s("y")],
    )
    .unwrap();
    assert_eq!(show(&mut realm, &spliced), "1,x,y,3");
    let replaced = call(&mut realm, &values, "with", &[n(-1.0), s("z")]).unwrap();
    assert_eq!(show(&mut realm, &replaced), "1,undefined,z");
    let replaced = call(&mut realm, &values, "with", &[n(0.0), s("w")]).unwrap();
    assert_eq!(show(&mut realm, &replaced), "w,undefined,3");
    assert_eq!(show(&mut realm, &values), "1,_,3");
    for index in [3.0, -4.0] {
        let thrown = call(&mut realm, &values, "with", &[n(index), s("z")]);
        assert_eq!(message(&mut realm, thrown), "RangeError: Invalid index");
    }
    let huge = array_like(&realm, 5e9, &[]);
    for name in ["toReversed", "toSorted"] {
        let thrown = call(&mut realm, &huge, name, &[]);
        assert_eq!(
            message(&mut realm, thrown),
            "RangeError: Invalid array length",
            "{name}"
        );
    }
    let thrown = call(&mut realm, &huge, "toSpliced", &[n(0.0), n(0.0)]);
    assert_eq!(
        message(&mut realm, thrown),
        "RangeError: Invalid array length"
    );
    let full = array_like(&realm, MAX_LENGTH as f64, &[]);
    let thrown = call(&mut realm, &full, "toSpliced", &[n(0.0), n(0.0), n(1.0)]);
    assert!(message(&mut realm, thrown).contains("exceeds the maximum length"));
}

/// An accessor property whose getter throws `RangeError: <name>`.
fn throwing_getter(realm: &Realm, name: &'static str) -> Property {
    let getter = realm.function("get", 0, false, move |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, name))
    });
    Property {
        slot: Slot::Accessor {
            get: Some(getter),
            set: None,
        },
        enumerable: true,
        configurable: true,
    }
}

#[test]
fn abrupt_callbacks_getters_and_writes_propagate() {
    let mut realm = Realm::new();
    let values = array(&realm, &[n(1.0), array(&realm, &[n(2.0)])]);
    let thrower = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "callback"))
    });
    for name in [
        "forEach",
        "every",
        "some",
        "map",
        "filter",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "flatMap",
        "reduce",
        "reduceRight",
    ] {
        let thrown = call(&mut realm, &values, name, std::slice::from_ref(&thrower));
        assert_eq!(
            message(&mut realm, thrown),
            "RangeError: callback",
            "{name}"
        );
    }
    let sum = function(&realm, |realm, _, args| ops::add(realm, &args[0], &args[1]));
    let guarded = realm.array(vec![]);
    guarded.define(Key::Index(0), throwing_getter(&realm, "element"));
    guarded.define(Key::Index(1), Property::data(n(2.0)));
    let guarded = Value::Object(guarded);
    for name in ["reduce", "reduceRight"] {
        let thrown = call(&mut realm, &guarded, name, std::slice::from_ref(&sum));
        assert_eq!(message(&mut realm, thrown), "RangeError: element", "{name}");
    }
    let nested = array(&realm, std::slice::from_ref(&guarded));
    let thrown = call(&mut realm, &nested, "flat", &[]);
    assert_eq!(message(&mut realm, thrown), "RangeError: element");
    for name in [
        "concat",
        "copyWithin",
        "slice",
        "toReversed",
        "with",
        "reverse",
    ] {
        let thrown = call(&mut realm, &guarded, name, &[n(0.0)]);
        assert!(message(&mut realm, thrown).contains("RangeError"), "{name}");
    }
    let thrown = call(&mut realm, &guarded, "sort", &[]);
    assert_eq!(message(&mut realm, thrown), "RangeError: element");
    let constructor = global(&mut realm, "Array");
    let thrown = statics(&mut realm, &constructor, "from", &[Value::Undefined]);
    assert!(message(&mut realm, thrown).starts_with("TypeError"));
    // A new target whose prototype cannot be read.
    let target = realm.function("Target", 0, true, |_, _, _| Ok(Value::Undefined));
    target.define(Key::name("prototype"), throwing_getter(&realm, "prototype"));
    let thrown = ops::construct_target(&mut realm, &constructor, &[], &Value::Object(target));
    assert_eq!(message(&mut realm, thrown), "RangeError: prototype");
    // Arrays that cannot grow reject pushed elements.
    let sealed = realm.array(vec![]);
    sealed.prevent_extensions();
    let thrown = call(&mut realm, &Value::Object(sealed), "push", &[n(1.0)]);
    assert!(message(&mut realm, thrown).starts_with("TypeError"));
    // Writing over a read-only element fails.
    let fixed = realm.array(vec![n(1.0), n(2.0)]);
    fixed.define(
        Key::Index(0),
        Property {
            slot: Slot::Data {
                value: n(1.0),
                writable: false,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let thrown = call(
        &mut realm,
        &Value::Object(fixed),
        "splice",
        &[n(0.0), n(1.0), s("x")],
    );
    assert!(message(&mut realm, thrown).starts_with("TypeError"));
}

#[test]
fn proxies_of_arrays_spread_and_flatten_as_arrays() {
    // Reactive arrays, as Vue's, are proxies of arrays: `IsArray` sees
    // through them.
    let mut realm = Realm::new();
    let target = array(&realm, &[n(1.0), n(2.0)]);
    let handler = Value::Object(realm.object());
    let constructor = global(&mut realm, "Proxy");
    let proxy = ops::construct(&mut realm, &constructor, &[target, handler]).unwrap();
    let first = array(&realm, &[n(0.0)]);
    let joined = call(&mut realm, &first, "concat", std::slice::from_ref(&proxy)).unwrap();
    assert_eq!(show(&mut realm, &joined), "0,1,2");
    let nested = array(&realm, std::slice::from_ref(&proxy));
    let flat = call(&mut realm, &nested, "flat", &[]).unwrap();
    assert_eq!(show(&mut realm, &flat), "1,2");
    let constructor = global(&mut realm, "Array");
    let checked = statics(
        &mut realm,
        &constructor,
        "isArray",
        std::slice::from_ref(&proxy),
    );
    assert!(matches!(checked, Ok(Value::Boolean(true))));
    // Methods called on one create plain arrays.
    let mapped = call(&mut realm, &proxy, "slice", &[]).unwrap();
    assert!(mapped.as_object().unwrap().is_array());
    assert_eq!(show(&mut realm, &mapped), "1,2");
}
