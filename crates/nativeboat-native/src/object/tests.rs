use super::*;

fn data(value: f64) -> Property {
    Property::data(Value::Number(value))
}

fn number(property: Option<Property>) -> Option<f64> {
    match property?.slot {
        Slot::Data {
            value: Value::Number(value),
            ..
        } => Some(value),
        _ => None,
    }
}

fn array(elements: Vec<Option<Value>>) -> Object {
    Object::new(None, Kind::Array(ArrayData::new(elements)))
}

fn length(object: &Object) -> Option<f64> {
    number(object.own_property(&Key::name("length")))
}

fn keys(object: &Object) -> Vec<String> {
    object
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect()
}

fn fixed(value: Value) -> Property {
    Property {
        slot: Slot::Data {
            value,
            writable: false,
        },
        enumerable: true,
        configurable: false,
    }
}

#[test]
fn keys_canonicalize_array_indexes() {
    assert_eq!(Key::name("0"), Key::Index(0));
    assert_eq!(Key::name("4294967294"), Key::Index(4_294_967_294));
    for text in ["4294967295", "99999999999", "01", "-1", "1.5", "", "a"] {
        assert_eq!(Key::name(text), Key::String(JsStr::from(text)), "{text}");
    }
    assert_eq!(Key::Index(7).to_js(), JsStr::from("7"));
    assert_eq!(Key::name("x").to_js(), JsStr::from("x"));
}

#[test]
fn ordinary_properties_keep_language_order_and_attributes() {
    let object = Object::new(None, Kind::Ordinary);
    assert!(format!("{object:?}").starts_with("Object(0x"));
    assert_eq!(object, object.clone());
    assert_ne!(object, Object::new(None, Kind::Ordinary));
    for key in ["b", "2", "a", "0"] {
        assert!(object.define(Key::name(key), data(1.0)));
    }
    let names: Vec<_> = object
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(names, ["0", "2", "b", "a"]);
    assert!(object.write_own(&Key::name("b"), Value::Number(5.0)));
    assert_eq!(number(object.own_property(&Key::name("b"))), Some(5.0));
    object.define(
        Key::name("fixed"),
        Property {
            slot: Slot::Data {
                value: Value::Null,
                writable: false,
            },
            enumerable: false,
            configurable: false,
        },
    );
    assert!(!object.write_own(&Key::name("fixed"), Value::Null));
    assert!(!object.delete(&Key::name("fixed")));
    object.define(
        Key::name("accessor"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    assert!(!object.write_own(&Key::name("accessor"), Value::Null));
    assert!(object.delete(&Key::name("missing")));
    object.data_mut().extensible = false;
    assert!(!object.write_own(&Key::name("new"), Value::Null));
    assert!(object.code().is_none());
    assert!(!object.is_array());
}

#[test]
fn removals_compact_and_keep_order() {
    let object = Object::new(None, Kind::Ordinary);
    for index in 0..6 {
        object.define(Key::name(&format!("k{index}")), data(f64::from(index)));
    }
    for index in [0, 2, 4, 5] {
        assert!(object.delete(&Key::name(&format!("k{index}"))));
    }
    let names: Vec<_> = object
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(names, ["k1", "k3"]);
    assert_eq!(number(object.own_property(&Key::name("k3"))), Some(3.0));
    object.define(Key::name("k1"), data(10.0));
    assert_eq!(number(object.own_property(&Key::name("k1"))), Some(10.0));
}

#[test]
fn arrays_store_dense_and_sparse_elements_with_their_attributes() {
    let list = array(vec![Some(Value::Number(1.0)), None]);
    assert!(list.is_array());
    assert_eq!(number(list.own_property(&Key::Index(0))), Some(1.0));
    assert!(list.own_property(&Key::Index(1)).is_none());
    assert!(list.own_property(&Key::Index(9)).is_none());
    assert_eq!(length(&list), Some(2.0));
    assert!(list.define(Key::name("extra"), data(3.0)));
    assert_eq!(keys(&list), ["0", "length", "extra"]);
    assert!(list.write_own(&Key::Index(3), Value::Number(4.0)));
    assert_eq!(length(&list), Some(4.0));
    // Non-default attributes and far indexes live as sparse properties.
    assert!(list.define(Key::Index(1), Property::hidden(Value::Null)));
    assert!(!list.own_property(&Key::Index(1)).unwrap().enumerable);
    assert!(list.define(Key::Index(1), data(2.0)));
    assert!(list.own_property(&Key::Index(1)).unwrap().enumerable);
    assert!(list.define(Key::Index(0), fixed(Value::Null)));
    assert!(!list.write_own(&Key::Index(0), Value::Number(9.0)));
    assert!(!list.delete(&Key::Index(0)));
    assert!(list.define(Key::Index(u32::MAX - 1), data(0.0)));
    assert_eq!(length(&list), Some(4_294_967_295.0));
    assert!(list.write_own(&Key::Index(5_000_000), Value::Null));
    assert_eq!(
        keys(&list),
        ["0", "1", "3", "5000000", "4294967294", "length", "extra"]
    );
    assert!(!list.define(Key::name("length"), data(0.0)));
    assert!(list.write_own(&Key::name("extra"), Value::Null));
    assert!(list.delete(&Key::Index(5_000_000)));
    assert!(list.delete(&Key::Index(1)));
    assert!(list.delete(&Key::Index(1)));
    assert!(list.delete(&Key::Index(40)));
    assert!(!list.delete(&Key::name("length")));
    assert!(list.delete(&Key::name("extra")));
    // Shrinking stops above a non-configurable element.
    assert!(!list.write_own(&Key::name("length"), Value::Number(0.0)));
    assert_eq!(length(&list), Some(1.0));
    for value in [Value::Number(1.5), Value::Number(-1.0), Value::Null] {
        assert!(!list.write_own(&Key::name("length"), value));
    }
    assert!(list.write_own(&Key::name("length"), Value::Number(3.0)));
    assert_eq!(length(&list), Some(3.0));
    list.prevent_extensions();
    assert!(!list.is_extensible());
    assert!(!list.write_own(&Key::Index(2), Value::Null));
    let shrink = array(vec![Some(Value::Null), Some(Value::Null)]);
    assert!(shrink.write_own(&Key::Index(1), Value::Number(2.0)));
    assert_eq!(number(shrink.own_property(&Key::Index(1))), Some(2.0));
    shrink.define(Key::name("named"), data(1.0));
    shrink.define(Key::Index(0), Property::hidden(Value::Null));
    assert!(shrink.write_own(&Key::name("length"), Value::Number(1.0)));
    assert_eq!(keys(&shrink), ["0", "length", "named"]);
}

#[test]
fn defining_properties_validates_redefinition() {
    let object = Object::new(None, Kind::Ordinary);
    let define = |key: &str, descriptor: Descriptor| {
        object.define_own_property(&Key::name(key), &descriptor)
    };
    let value = |value: f64| Some(Value::Number(value));
    assert!(define(
        "plain",
        Descriptor {
            value: value(1.0),
            ..Descriptor::default()
        }
    ));
    let created = object.own_property(&Key::name("plain")).unwrap();
    assert!(!created.enumerable && !created.configurable);
    assert!(matches!(
        created.slot,
        Slot::Data {
            writable: false,
            ..
        }
    ));
    // Non-configurable, non-writable: only identical values are accepted.
    assert!(define(
        "plain",
        Descriptor {
            value: value(1.0),
            writable: Some(false),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "plain",
        Descriptor {
            value: value(2.0),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "plain",
        Descriptor {
            writable: Some(true),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "plain",
        Descriptor {
            configurable: Some(true),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "plain",
        Descriptor {
            enumerable: Some(true),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "plain",
        Descriptor {
            get: Some(None),
            ..Descriptor::default()
        }
    ));
    assert!(define(
        "plain",
        Descriptor {
            enumerable: Some(false),
            ..Descriptor::default()
        }
    ));
    let nan = Descriptor {
        value: Some(Value::Number(f64::NAN)),
        writable: Some(true),
        configurable: Some(false),
        ..Descriptor::default()
    };
    assert!(define("nan", nan.clone()));
    assert!(define(
        "nan",
        Descriptor {
            value: Some(Value::Number(f64::NAN)),
            writable: Some(false),
            ..Descriptor::default()
        }
    ));
    assert!(define(
        "nan",
        Descriptor {
            value: Some(Value::Number(f64::NAN)),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "nan",
        Descriptor {
            value: Some(Value::Number(0.0)),
            ..Descriptor::default()
        }
    ));
    // Accessors keep their functions unless the property is configurable.
    let getter = Object::new(None, Kind::Ordinary);
    assert!(define(
        "accessor",
        Descriptor {
            get: Some(Some(getter.clone())),
            ..Descriptor::default()
        }
    ));
    assert!(matches!(
        object.own_property(&Key::name("accessor")).unwrap().slot,
        Slot::Accessor { set: None, .. }
    ));
    assert!(define(
        "accessor",
        Descriptor {
            get: Some(Some(getter.clone())),
            set: Some(None),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "accessor",
        Descriptor {
            get: Some(None),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "accessor",
        Descriptor {
            set: Some(Some(getter.clone())),
            ..Descriptor::default()
        }
    ));
    assert!(!define(
        "accessor",
        Descriptor {
            value: value(1.0),
            ..Descriptor::default()
        }
    ));
    // Configurable properties switch between data and accessor forms.
    assert!(define(
        "flexible",
        Descriptor {
            value: value(1.0),
            configurable: Some(true),
            enumerable: Some(true),
            ..Descriptor::default()
        }
    ));
    assert!(define(
        "flexible",
        Descriptor {
            set: Some(Some(getter.clone())),
            ..Descriptor::default()
        }
    ));
    let switched = object.own_property(&Key::name("flexible")).unwrap();
    assert!(switched.enumerable && switched.configurable);
    assert!(matches!(
        switched.slot,
        Slot::Accessor {
            get: None,
            set: Some(_)
        }
    ));
    assert!(define(
        "flexible",
        Descriptor {
            writable: Some(true),
            ..Descriptor::default()
        }
    ));
    assert!(matches!(
        object.own_property(&Key::name("flexible")).unwrap().slot,
        Slot::Data {
            value: Value::Undefined,
            writable: true
        }
    ));
    assert!(define(
        "flexible",
        Descriptor {
            value: value(5.0),
            ..Descriptor::default()
        }
    ));
    assert_eq!(
        number(object.own_property(&Key::name("flexible"))),
        Some(5.0)
    );
    assert!(define(
        "flexible",
        Descriptor {
            get: Some(None),
            ..Descriptor::default()
        }
    ));
    assert!(define(
        "flexible",
        Descriptor {
            set: Some(None),
            ..Descriptor::default()
        }
    ));
    object.prevent_extensions();
    assert!(!define("fresh", Descriptor::default()));
    assert!(Descriptor::of(&switched).is_accessor());
    assert!(Descriptor::of(&created).is_data());
}

#[test]
fn array_lengths_and_indexes_follow_length_writability() {
    let list = array(vec![
        Some(Value::Null),
        Some(Value::Null),
        Some(Value::Null),
    ]);
    let length_to = |value: Option<Value>, writable: Option<bool>| Descriptor {
        value,
        writable,
        ..Descriptor::default()
    };
    let key = Key::name("length");
    assert!(list.define_own_property(&key, &length_to(Some(Value::Number(3.0)), None)));
    assert!(list.define_own_property(&key, &length_to(Some(Value::Number(2.0)), None)));
    assert_eq!(length(&list), Some(2.0));
    for refused in [
        Descriptor {
            configurable: Some(true),
            ..Descriptor::default()
        },
        Descriptor {
            enumerable: Some(true),
            ..Descriptor::default()
        },
        Descriptor {
            get: Some(None),
            ..Descriptor::default()
        },
    ] {
        assert!(!list.define_own_property(&key, &refused));
    }
    assert!(list.define_own_property(&key, &length_to(Some(Value::Number(1.0)), Some(false))));
    assert_eq!(length(&list), Some(1.0));
    assert!(!list.define_own_property(&key, &length_to(None, Some(true))));
    assert!(!list.define_own_property(&key, &length_to(Some(Value::Number(4.0)), None)));
    assert!(list.define_own_property(&key, &length_to(Some(Value::Number(1.0)), None)));
    assert!(!list.write_own(&key, Value::Number(5.0)));
    let past_end = Descriptor {
        value: Some(Value::Null),
        writable: Some(true),
        enumerable: Some(true),
        configurable: Some(true),
        ..Descriptor::default()
    };
    assert!(!list.define_own_property(&Key::Index(3), &past_end));
    assert!(!list.write_own(&Key::Index(3), Value::Null));
    assert!(list.define_own_property(&Key::Index(0), &past_end));
}

#[test]
fn string_wrappers_expose_read_only_units() {
    let text = Object::new(None, Kind::Primitive(Wrapped::String(JsStr::from("ab"))));
    assert_eq!(keys(&text), ["0", "1", "length"]);
    text.define(Key::Index(5), data(1.0));
    text.define(Key::name("extra"), data(1.0));
    assert_eq!(keys(&text), ["0", "1", "5", "length", "extra"]);
    let unit = text.own_property(&Key::Index(1)).unwrap();
    assert!(unit.enumerable && !unit.configurable);
    assert!(
        matches!(unit.slot, Slot::Data { value: Value::String(ref value), writable: false } if value == &JsStr::from("b"))
    );
    assert_eq!(length(&text), Some(2.0));
    assert!(text.own_property(&Key::Index(2)).is_none());
    assert!(!text.define(Key::Index(0), data(0.0)));
    assert!(!text.define(Key::name("length"), data(0.0)));
    assert!(!text.write_own(&Key::Index(0), Value::Null));
    assert!(!text.delete(&Key::Index(0)));
    assert!(text.delete(&Key::Index(5)));
    let same = Descriptor {
        value: Some(Value::str("a")),
        ..Descriptor::default()
    };
    assert!(text.define_own_property(&Key::Index(0), &same));
    let other = Descriptor {
        value: Some(Value::str("z")),
        ..Descriptor::default()
    };
    assert!(!text.define_own_property(&Key::Index(0), &other));
    let number = Object::new(None, Kind::Primitive(Wrapped::Number(1.0)));
    let truth = Object::new(None, Kind::Primitive(Wrapped::Boolean(true)));
    assert!(number.own_keys().is_empty() && truth.own_keys().is_empty());
}

#[test]
fn prototypes_change_without_cycles_or_after_preventing_extensions() {
    let parent = Object::new(None, Kind::Ordinary);
    let child = Object::new(Some(parent.clone()), Kind::Ordinary);
    assert!(child.set_prototype(Some(parent.clone())));
    assert!(!parent.set_prototype(Some(child.clone())));
    assert!(!child.set_prototype(Some(child.clone())));
    assert!(child.set_prototype(None));
    assert!(child.data().prototype.is_none());
    child.prevent_extensions();
    assert!(!child.set_prototype(Some(parent)));
    assert!(child.set_prototype(None));
}

#[test]
fn same_value_distinguishes_zeros_and_equates_nan() {
    let symbol = crate::Symbol::new(None);
    let object = Object::new(None, Kind::Ordinary);
    for (left, right, same) in [
        (Value::Number(f64::NAN), Value::Number(f64::NAN), true),
        (Value::Number(0.0), Value::Number(-0.0), false),
        (Value::Number(1.0), Value::Number(1.0), true),
        (Value::Undefined, Value::Undefined, true),
        (Value::Null, Value::Null, true),
        (Value::Boolean(true), Value::Boolean(true), true),
        (Value::str("a"), Value::str("a"), true),
        (Value::Symbol(symbol.clone()), Value::Symbol(symbol), true),
        (Value::Object(object.clone()), Value::Object(object), true),
        (Value::Null, Value::Undefined, false),
    ] {
        assert_eq!(same_value(&left, &right), same, "{left:?} {right:?}");
    }
}

#[test]
fn functions_expose_their_code() {
    let function = Object::new(
        None,
        Kind::Function {
            code: Rc::new(|_, _, _| Ok(Value::Number(7.0))),
            constructor: true,
            realm: 0,
        },
    );
    let (code, constructor) = function.code().unwrap();
    assert!(constructor);
    let mut realm = Realm::new();
    assert!(matches!(
        code(&mut realm, Value::Undefined, &[]),
        Ok(Value::Number(7.0))
    ));
}

#[test]
fn object_state_formats_for_diagnostics() {
    let kinds = [
        Kind::Ordinary,
        Kind::Array(ArrayData::new(vec![None, None])),
        Kind::Primitive(Wrapped::Number(1.0)),
        Kind::Function {
            code: Rc::new(|_, _, _| Ok(Value::Undefined)),
            constructor: false,
            realm: 0,
        },
        Kind::Error,
    ];
    let texts: Vec<_> = kinds
        .into_iter()
        .map(|kind| {
            let object = Object::new(None, kind);
            object.define(Key::name("p"), Property::data(Value::Null));
            format!("{:?}", *object.data())
        })
        .collect();
    assert!(texts[0].contains("kind: Ordinary"));
    assert!(texts[1].contains("kind: Array(2)"));
    assert!(texts[2].contains("kind: Primitive(Number(1.0))"));
    assert!(texts[3].contains("kind: Function(false)"));
    assert!(texts[4].contains("kind: Error"));
    assert!(texts[0].contains("properties: Properties"));
}

#[test]
fn node_private_keys_are_hidden_own_only_and_survive_prevent_extensions() {
    let mut realm = Realm::new();
    let symbol = Symbol::private("node:transfer_mode");
    let weak = symbol.downgrade();
    assert_eq!(weak.upgrade().unwrap(), symbol);
    assert_eq!(
        symbol.descriptive(),
        JsStr::from("Symbol(node:transfer_mode)")
    );
    let key = Key::Symbol(symbol);
    let parent = realm.object();
    parent.prevent_extensions();
    let value = Value::Object(parent.clone());
    crate::ops::set(&mut realm, &value, &key, Value::Number(3.0)).unwrap();
    assert_eq!(number(parent.own_property(&key)), Some(3.0));
    assert!(!parent.own_property(&key).unwrap().enumerable);
    assert!(parent.own_keys().is_empty());
    let child = Value::Object(Object::new(Some(parent.clone()), Kind::Ordinary));
    assert!(matches!(
        crate::ops::get(&mut realm, &child, &key).unwrap(),
        Value::Undefined
    ));
    assert!(!crate::ops::has_property(&mut realm, &key, &child).unwrap());
    assert!(crate::ops::has_property(&mut realm, &key, &value).unwrap());
    crate::ops::set(&mut realm, &child, &key, Value::Number(2.0)).unwrap();
    assert_eq!(number(parent.own_property(&key)), Some(3.0));
    assert!(crate::ops::delete(&mut realm, &child, &key).unwrap());
    assert!(crate::ops::delete(&mut realm, &value, &key).unwrap());
    assert!(!parent.define_own_property(&key, &Descriptor::default()));
    crate::ops::set(&mut realm, &value, &key, Value::Number(3.0)).unwrap();
    assert!(parent.define_own_property(
        &key,
        &Descriptor {
            value: Some(Value::Number(4.0)),
            writable: Some(false),
            enumerable: Some(true),
            configurable: Some(false),
            ..Descriptor::default()
        }
    ));
    assert!(!parent.own_property(&key).unwrap().enumerable);
    assert!(parent.define_own_property(
        &key,
        &Descriptor {
            enumerable: Some(true),
            ..Descriptor::default()
        }
    ));
    assert!(crate::ops::set(&mut realm, &value, &key, Value::Number(1.0)).is_err());
    assert!(crate::ops::delete(&mut realm, &value, &key).is_err());
    assert_eq!(number(parent.own_property(&key)), Some(4.0));
    assert!(parent.own_keys().is_empty());
}

#[test]
fn many_properties_are_found_by_fingerprint_through_removal_and_growth() {
    let object = Object::new(None, Kind::Ordinary);
    let symbol = crate::Symbol::new(None);
    let name = |index: usize| Key::String(JsStr::from(alloc::format!("p{index}").as_str()));
    for index in 0..40 {
        object.define(name(index), data(index as f64));
    }
    object.define(Key::Index(7), data(-7.0));
    object.define(Key::Symbol(symbol.clone()), data(-1.0));
    for index in 0..40 {
        assert_eq!(
            number(object.own_property(&name(index))),
            Some(index as f64)
        );
    }
    // An equal key from another buffer, and a view of a longer string.
    let sliced = JsStr::from("xp39").slice(1, 4);
    assert_eq!(
        number(object.own_property(&Key::String(sliced))),
        Some(39.0)
    );
    assert_eq!(number(object.own_property(&Key::Index(7))), Some(-7.0));
    assert_eq!(
        number(object.own_property(&Key::Symbol(symbol.clone()))),
        Some(-1.0)
    );
    assert!(
        object
            .own_property(&Key::Symbol(crate::Symbol::new(None)))
            .is_none()
    );
    assert!(object.own_property(&name(40)).is_none());
    // Removing keeps the rest findable; a removed key may come back.
    for index in (0..40).step_by(3) {
        assert!(object.delete(&name(index)));
    }
    assert!(object.own_property(&name(3)).is_none());
    assert_eq!(number(object.own_property(&name(4))), Some(4.0));
    object.define(name(3), data(33.0));
    assert_eq!(number(object.own_property(&name(3))), Some(33.0));
    // Removing most slots compacts them, down to a scan.
    for index in 0..40 {
        object.delete(&name(index));
    }
    assert_eq!(keys(&object), ["7", "Symbol()"]);
    assert_eq!(
        number(object.own_property(&Key::Symbol(symbol))),
        Some(-1.0)
    );
    object.define(name(1), data(1.0));
    assert_eq!(keys(&object), ["7", "p1", "Symbol()"]);
}

#[test]
fn sized_objects_hold_more_properties_than_their_room() {
    let realm = crate::Realm::new();
    let object = realm.object_sized(2);
    assert_eq!(object.data().properties.entries.capacity(), 2);
    for (index, name) in ["a", "b", "c"].iter().enumerate() {
        object.define(Key::name(name), data(index as f64));
    }
    assert_eq!(keys(&object), ["a", "b", "c"]);
    assert_eq!(number(object.own_property(&Key::name("c"))), Some(2.0));
}
