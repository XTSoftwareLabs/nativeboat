use super::*;

#[test]
fn iterators_retain_the_collection_until_dropped_or_exhausted() {
    let mut realm = Realm::new();
    for exhaust in [false, true] {
        let collection = make(&mut realm, false, None);
        let weak = collection.as_object().unwrap().downgrade();
        let iterator = call(&mut realm, &collection, "keys", &[]).unwrap();
        drop(collection);
        assert!(weak.upgrade().is_some());
        if exhaust {
            assert!(iter::collect(&mut realm, &iterator).unwrap().is_empty());
            assert!(weak.upgrade().is_none());
        }
        drop(iterator);
        assert!(weak.upgrade().is_none());
    }
}

fn global(realm: &Realm, set: bool) -> Value {
    match realm
        .global
        .own_property(&Key::name(name(set)))
        .unwrap()
        .slot
    {
        Slot::Data { value, .. } => value,
        _ => unreachable!(),
    }
}

fn make(realm: &mut Realm, set: bool, source: Option<Value>) -> Value {
    let constructor = global(realm, set);
    ops::construct(realm, &constructor, &source.into_iter().collect::<Vec<_>>()).unwrap()
}

fn call(realm: &mut Realm, this: &Value, method: &str, args: &[Value]) -> Completion {
    let function = ops::get(realm, this, &Key::name(method)).unwrap();
    ops::call(realm, &function, this.clone(), args)
}

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

fn size(realm: &mut Realm, value: &Value) -> f64 {
    match ops::get(realm, value, &Key::name("size")).unwrap() {
        Value::Number(size) => size,
        _ => unreachable!(),
    }
}

fn items(realm: &mut Realm, collection: &Value, method: &str) -> Vec<Value> {
    let iterator = call(realm, collection, method, &[]).unwrap();
    iter::collect(realm, &iterator).unwrap()
}

#[test]
fn keyed_lookup_uses_same_value_zero_without_coercing_keys() {
    let mut realm = Realm::new();
    for set in [false, true] {
        let collection = make(&mut realm, set, None);
        let symbol = Value::Symbol(crate::Symbol::new(Some("same".into())));
        let object = realm.object();
        let coercion = realm.function("valueOf", 0, false, |realm, _, _| {
            Err(realm.throw(ErrorKind::Error, "coercion"))
        });
        object.define(
            Key::name("valueOf"),
            Property::data(Value::Object(coercion)),
        );
        let object = Value::Object(object);
        let keys = [
            Value::Undefined,
            Value::Null,
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Number(-0.0),
            Value::Number(f64::NAN),
            Value::Number(1.0),
            Value::Number(f64::INFINITY),
            Value::String(crate::JsStr::from_units(&[0xD800])),
            Value::str("1"),
            symbol.clone(),
            object.clone(),
        ];
        for (index, key) in keys.iter().enumerate() {
            let returned = call(
                &mut realm,
                &collection,
                if set { "add" } else { "set" },
                &[key.clone(), Value::Number(index as f64)],
            )
            .unwrap();
            assert!(crate::object::same_value(&returned, &collection));
            assert!(ops::to_boolean(
                &call(&mut realm, &collection, "has", core::slice::from_ref(key)).unwrap()
            ));
        }
        assert_eq!(size(&mut realm, &collection), 12.0);
        let mut equivalents = keys.to_vec();
        equivalents[4] = Value::Number(0.0);
        equivalents[5] = Value::Number(f64::from_bits(0x7FF8000000000001));
        for (index, key) in equivalents.iter().enumerate() {
            assert!(ops::to_boolean(
                &call(&mut realm, &collection, "has", core::slice::from_ref(key)).unwrap()
            ));
            if !set {
                let got = call(&mut realm, &collection, "get", core::slice::from_ref(key)).unwrap();
                assert!(crate::object::same_value(
                    &got,
                    &Value::Number(index as f64)
                ));
            }
        }
        for key in [
            Value::Object(realm.object()),
            Value::Symbol(crate::Symbol::new(Some("same".into()))),
            Value::str("missing"),
        ] {
            assert!(!ops::to_boolean(
                &call(&mut realm, &collection, "has", core::slice::from_ref(&key)).unwrap()
            ));
            if !set {
                assert!(matches!(
                    call(&mut realm, &collection, "get", &[key]).unwrap(),
                    Value::Undefined
                ));
            }
        }
        call(
            &mut realm,
            &collection,
            if set { "add" } else { "set" },
            &[Value::Number(0.0), Value::str("updated")],
        )
        .unwrap();
        assert_eq!(size(&mut realm, &collection), 12.0);
        let keys = items(&mut realm, &collection, "keys");
        assert!(matches!(keys[4],Value::Number(number) if number.to_bits()==0));
        if !set {
            let got = call(&mut realm, &collection, "get", &[Value::Number(-0.0)]).unwrap();
            assert_eq!(text(&mut realm, &got), "updated");
        }
        let data = receiver(&realm, &collection, set).unwrap();
        assert_eq!(data.snapshot(100).len(), 12);
        assert!(
            format!("{:?}", collection.as_object().unwrap().data().kind).contains("Collection")
        );
        for key in keys {
            assert!(ops::to_boolean(
                &call(&mut realm, &collection, "delete", &[key]).unwrap()
            ));
        }
        assert_eq!(size(&mut realm, &collection), 0.0);
        assert!(!ops::to_boolean(
            &call(&mut realm, &collection, "delete", &[Value::Null]).unwrap()
        ));
        assert!(data.data.borrow().entries.is_empty());
    }
}

#[test]
fn live_order_tracks_updates_deletes_clear_and_append() {
    let mut realm = Realm::new();
    for set in [false, true] {
        let collection = make(&mut realm, set, None);
        for key in ["a", "b", "c"] {
            call(
                &mut realm,
                &collection,
                if set { "add" } else { "set" },
                &[Value::str(key), Value::str(key)],
            )
            .unwrap();
        }
        let values = call(&mut realm, &collection, "values", &[]).unwrap();
        let step = call(&mut realm, &values, "next", &[]).unwrap();
        let first = ops::get(&mut realm, &step, &Key::name("value")).unwrap();
        assert_eq!(text(&mut realm, &first), "a");
        call(&mut realm, &collection, "delete", &[Value::str("b")]).unwrap();
        call(
            &mut realm,
            &collection,
            if set { "add" } else { "set" },
            &[Value::str("c"), Value::str("updated")],
        )
        .unwrap();
        call(
            &mut realm,
            &collection,
            if set { "add" } else { "set" },
            &[Value::str("b"), Value::str("back")],
        )
        .unwrap();
        let rest = iter::collect(&mut realm, &values).unwrap();
        let rest: Vec<_> = rest.iter().map(|value| text(&mut realm, value)).collect();
        assert_eq!(
            rest,
            if set {
                vec!["c", "b"]
            } else {
                vec!["updated", "back"]
            }
        );
        call(
            &mut realm,
            &collection,
            if set { "add" } else { "set" },
            &[Value::str("later")],
        )
        .unwrap();
        assert!(iter::collect(&mut realm, &values).unwrap().is_empty());
        let entries = items(&mut realm, &collection, "entries");
        assert_eq!(text(&mut realm, &entries[0]), "a,a");
        let live = call(&mut realm, &collection, "entries", &[]).unwrap();
        call(&mut realm, &collection, "clear", &[]).unwrap();
        call(
            &mut realm,
            &collection,
            if set { "add" } else { "set" },
            &[Value::str("fresh"), Value::str("value")],
        )
        .unwrap();
        let rest = iter::collect(&mut realm, &live).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(
            text(&mut realm, &rest[0]),
            if set { "fresh,fresh" } else { "fresh,value" }
        );
        call(&mut realm, &collection, "clear", &[]).unwrap();
        call(&mut realm, &collection, "clear", &[]).unwrap();
        let data = receiver(&realm, &collection, set).unwrap();
        assert!(data.snapshot(100).is_empty());
        assert!(data.data.borrow().entries.is_empty());
        assert!(
            format!("{:?}", live.as_object().unwrap().data().kind).contains("CollectionIterator")
        );
    }
}

#[test]
fn leases_reclaim_tombstones_on_exhaustion_drop_and_callback_errors() {
    let mut realm = Realm::new();
    let collection = make(&mut realm, false, None);
    let data = receiver(&realm, &collection, false).unwrap();
    for index in 0..100 {
        call(
            &mut realm,
            &collection,
            "set",
            &[Value::Number(index as f64), Value::Number(index as f64)],
        )
        .unwrap();
    }
    let first = call(&mut realm, &collection, "keys", &[]).unwrap();
    let second = call(&mut realm, &collection, "keys", &[]).unwrap();
    for index in 0..99 {
        call(
            &mut realm,
            &collection,
            "delete",
            &[Value::Number(index as f64)],
        )
        .unwrap();
    }
    assert_eq!(data.data.borrow().entries.len(), 100);
    drop(first);
    assert_eq!(data.data.borrow().active, 1);
    let rest = iter::collect(&mut realm, &second).unwrap();
    assert_eq!(text(&mut realm, &rest[0]), "99");
    assert_eq!(data.data.borrow().entries.len(), 1);
    assert_eq!(data.data.borrow().entries.capacity(), 1);
    let callback = realm.function("callback", 1, false, |realm, this, args| {
        assert!(matches!(this, Value::Number(7.0)));
        let collection = args[2].clone();
        call(realm, &collection, "clear", &[]).unwrap();
        call(
            realm,
            &collection,
            "set",
            &[Value::str("new"), Value::str("new")],
        )
        .unwrap();
        Err(realm.throw(ErrorKind::Error, "callback"))
    });
    let error = call(
        &mut realm,
        &collection,
        "forEach",
        &[Value::Object(callback), Value::Number(7.0)],
    )
    .unwrap_err();
    assert_eq!(text(&mut realm, &error.0), "Error: callback");
    assert_eq!(data.data.borrow().active, 0);
    assert_eq!(data.data.borrow().entries.len(), 2);
    call(&mut realm, &collection, "delete", &[Value::str("new")]).unwrap();
    assert!(data.data.borrow().entries.is_empty());
    // Removing the last reference to an iterator stored as a value must
    // release its lease after the mutable collection borrow ends.
    let iterator = call(&mut realm, &collection, "keys", &[]).unwrap();
    call(
        &mut realm,
        &collection,
        "set",
        &[Value::str("iterator"), iterator],
    )
    .unwrap();
    call(
        &mut realm,
        &collection,
        "set",
        &[Value::str("iterator"), Value::Null],
    )
    .unwrap();
    assert_eq!(data.data.borrow().active, 0);
    let iterator = call(&mut realm, &collection, "keys", &[]).unwrap();
    call(
        &mut realm,
        &collection,
        "set",
        &[Value::str("iterator"), iterator],
    )
    .unwrap();
    call(&mut realm, &collection, "delete", &[Value::str("iterator")]).unwrap();
    assert_eq!(data.data.borrow().active, 0);
    let iterator = call(&mut realm, &collection, "keys", &[]).unwrap();
    call(
        &mut realm,
        &collection,
        "set",
        &[Value::str("iterator"), iterator],
    )
    .unwrap();
    call(&mut realm, &collection, "clear", &[]).unwrap();
    assert_eq!(data.data.borrow().active, 0);
}

#[test]
fn foreach_observes_live_mutations_and_same_set_callback_values() {
    let mut realm = Realm::new();
    for set in [false, true] {
        let collection = make(&mut realm, set, None);
        call(
            &mut realm,
            &collection,
            if set { "add" } else { "set" },
            &[Value::str("a"), Value::str("A")],
        )
        .unwrap();
        call(
            &mut realm,
            &collection,
            if set { "add" } else { "set" },
            &[Value::str("skip"), Value::str("skip")],
        )
        .unwrap();
        let log = Rc::new(RefCell::new(Vec::new()));
        let output = log.clone();
        let callback = realm.function("callback", 3, false, move |realm, this, args| {
            assert!(matches!(this, Value::Boolean(true)));
            output.borrow_mut().push(format!(
                "{}:{}",
                text(realm, &args[0]),
                text(realm, &args[1])
            ));
            if text(realm, &args[1]) == "a" {
                call(realm, &args[2], "delete", &[Value::str("skip")]).unwrap();
                call(
                    realm,
                    &args[2],
                    if set { "add" } else { "set" },
                    &[Value::str("b"), Value::str("B")],
                )
                .unwrap();
            }
            Ok(Value::Undefined)
        });
        call(
            &mut realm,
            &collection,
            "forEach",
            &[Value::Object(callback), Value::Boolean(true)],
        )
        .unwrap();
        assert_eq!(
            *log.borrow(),
            if set {
                vec!["a:a", "b:b"]
            } else {
                vec!["A:a", "B:b"]
            }
        );
        assert!(call(&mut realm, &collection, "forEach", &[Value::Null]).is_err());
    }
}

#[test]
fn constructors_descriptors_brand_checks_and_iterators_match_intrinsics() {
    let mut realm = Realm::new();
    for set in [false, true] {
        let constructor = global(&realm, set);
        assert!(ops::call(&mut realm, &constructor, Value::Undefined, &[]).is_err());
        assert!(matches!(
            ops::get(&mut realm, &constructor, &Key::name("length")).unwrap(),
            Value::Number(0.0)
        ));
        let prototype = ops::get(&mut realm, &constructor, &Key::name("prototype")).unwrap();
        let species_key = Key::Symbol(realm.symbols.species.clone());
        let species = ops::get(&mut realm, &constructor, &species_key).unwrap();
        assert!(crate::object::same_value(&species, &constructor));
        let collection = make(&mut realm, set, Some(Value::Null));
        assert_eq!(
            text(&mut realm, &collection),
            format!("[object {}]", name(set))
        );
        for (method, length) in [
            (if set { "add" } else { "set" }, if set { 1.0 } else { 2.0 }),
            ("has", 1.0),
            ("delete", 1.0),
            ("clear", 0.0),
            ("forEach", 1.0),
            ("keys", 0.0),
            ("values", 0.0),
            ("entries", 0.0),
        ] {
            let function = ops::get(&mut realm, &prototype, &Key::name(method)).unwrap();
            assert!(
                matches!(ops::get(&mut realm,&function,&Key::name("length")).unwrap(),Value::Number(value) if value==length)
            );
            for receiver in [Value::Null, prototype.clone(), make(&mut realm, !set, None)] {
                assert!(ops::call(&mut realm, &function, receiver, &[]).is_err());
            }
            assert!(
                !prototype
                    .as_object()
                    .unwrap()
                    .own_property(&Key::name(method))
                    .unwrap()
                    .enumerable
            );
        }
        assert!(ops::get(&mut realm, &prototype, &Key::name("size")).is_err());
        if !set {
            let get = ops::get(&mut realm, &prototype, &Key::name("get")).unwrap();
            assert!(ops::call(&mut realm, &get, Value::Number(0.0), &[]).is_err());
        }
        let values = ops::get(&mut realm, &prototype, &Key::name("values")).unwrap();
        let keys = ops::get(&mut realm, &prototype, &Key::name("keys")).unwrap();
        let entries = ops::get(&mut realm, &prototype, &Key::name("entries")).unwrap();
        let iterator_key = Key::Symbol(realm.symbols.iterator.clone());
        let default = ops::get(&mut realm, &prototype, &iterator_key).unwrap();
        assert!(crate::object::same_value(
            &default,
            if set { &values } else { &entries }
        ));
        assert_eq!(crate::object::same_value(&keys, &values), set);
        let iterator = call(&mut realm, &collection, "entries", &[]).unwrap();
        assert_eq!(
            text(&mut realm, &iterator),
            format!("[object {} Iterator]", name(set))
        );
        let next = ops::get(&mut realm, &iterator, &Key::name("next")).unwrap();
        assert!(ops::call(&mut realm, &next, Value::Null, &[]).is_err());
        let other = make(&mut realm, !set, None);
        let other = call(&mut realm, &other, "entries", &[]).unwrap();
        assert!(ops::call(&mut realm, &next, other, &[]).is_err());
        let target = realm.function("target", 0, true, |_, _, _| Ok(Value::Undefined));
        target.define(Key::name("prototype"), Property::data(Value::Null));
        let constructed =
            ops::construct_target(&mut realm, &constructor, &[], &Value::Object(target)).unwrap();
        assert_eq!(
            constructed.as_object().unwrap().data().prototype,
            prototype.as_object().cloned()
        );
    }
}

#[test]
fn construction_reads_entry_properties_and_closes_on_consumer_errors() {
    let mut realm = Realm::new();
    for set in [false, true] {
        let source = if set {
            Value::str("a😀a")
        } else {
            let pair = Value::Object(realm.array(vec![Value::str("a"), Value::Number(1.0)]));
            Value::Object(realm.array(vec![pair.clone(), pair]))
        };
        let collection = make(&mut realm, set, Some(source));
        assert_eq!(size(&mut realm, &collection), if set { 2.0 } else { 1.0 });
        let copied = make(&mut realm, set, Some(collection));
        assert_eq!(size(&mut realm, &copied), if set { 2.0 } else { 1.0 });
        let constructor = global(&realm, set);
        assert!(ops::construct(&mut realm, &constructor, &[Value::Number(1.0)]).is_err());
        for mode in [
            "bad entry",
            "key getter",
            "value getter",
            "adder",
            "iterator",
        ] {
            if set && mode != "adder" && mode != "iterator" {
                continue;
            }
            let closed = Rc::new(core::cell::Cell::new(0));
            let count = closed.clone();
            let source = realm.object();
            let iterator = realm.object();
            let returned = realm.function("return", 0, false, move |_, _, _| {
                count.set(count.get() + 1);
                Err(Thrown(Value::str("close")))
            });
            iterator.define(Key::name("return"), Property::data(Value::Object(returned)));
            let next = realm.function("next", 0, false, move |realm, _, _| {
                if mode == "iterator" {
                    return Err(Thrown(Value::str("iterator")));
                }
                let item = if mode == "bad entry" {
                    Value::Null
                } else {
                    let item = realm.object();
                    for index in [0, 1] {
                        if (mode == "key getter" && index == 0)
                            || (mode == "value getter" && index == 1)
                        {
                            let get = realm.function("get", 0, false, move |_, _, _| {
                                Err(Thrown(Value::str(mode)))
                            });
                            ops::define_accessor(&item, Key::Index(index), Some(get), None);
                        } else {
                            item.define(Key::Index(index), Property::data(Value::str("ok")));
                        }
                    }
                    Value::Object(item)
                };
                let result = realm.object();
                result.define(Key::name("value"), Property::data(item));
                result.define(Key::name("done"), Property::data(Value::Boolean(false)));
                Ok(Value::Object(result))
            });
            iterator.define(Key::name("next"), Property::data(Value::Object(next)));
            let method = realm.function("iterator", 0, false, move |_, _, _| {
                Ok(Value::Object(iterator.clone()))
            });
            source.define(
                Key::Symbol(realm.symbols.iterator.clone()),
                Property::data(Value::Object(method)),
            );
            let target = realm.function("target", 0, true, |_, _, _| Ok(Value::Undefined));
            let prototype = ops::get(&mut realm, &constructor, &Key::name("prototype")).unwrap();
            let prototype = Object::new(prototype.as_object().cloned(), Kind::Ordinary);
            if mode == "adder" {
                let adder = realm.function("adder", 1, false, |_, _, _| {
                    Err(Thrown(Value::str("adder")))
                });
                prototype.define(
                    Key::name(if set { "add" } else { "set" }),
                    Property::data(Value::Object(adder)),
                );
            }
            target.define(
                Key::name("prototype"),
                Property::data(Value::Object(prototype)),
            );
            let error = ops::construct_target(
                &mut realm,
                &constructor,
                &[Value::Object(source)],
                &Value::Object(target),
            )
            .unwrap_err();
            assert_eq!(closed.get(), if mode == "iterator" { 0 } else { 1 });
            let message = text(&mut realm, &error.0);
            assert!(
                message.contains(if mode == "bad entry" {
                    "entry object"
                } else {
                    mode
                }),
                "{message}"
            );
        }
        let prototype = realm.object();
        prototype.define(
            Key::name(if set { "add" } else { "set" }),
            Property::data(Value::Null),
        );
        let target = realm.function("target", 0, true, |_, _, _| Ok(Value::Undefined));
        target.define(
            Key::name("prototype"),
            Property::data(Value::Object(prototype)),
        );
        assert!(
            ops::construct_target(
                &mut realm,
                &constructor,
                &[Value::str("")],
                &Value::Object(target)
            )
            .is_err()
        );
    }
}
