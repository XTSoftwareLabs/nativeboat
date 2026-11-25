use super::*;
use crate::JsStr;

macro_rules! same {
    ($left:expr, $right:expr $(,)?) => {
        assert!(ops::strict_equals(&$left, &$right))
    };
}

fn message<T: std::fmt::Debug>(realm: &mut Realm, result: Result<T, Thrown>) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

fn get(realm: &mut Realm, value: &Value, key: &str) -> Value {
    ops::get(realm, value, &Key::name(key)).unwrap()
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_rust_lossy(),
        other => format!("{other:?}"),
    }
}

/// A base class whose construction records its arguments' count on `this`.
fn base(realm: &mut Realm) -> Object {
    let (_, parent) = heritage(realm, None).unwrap();
    let prototype = realm.object();
    create(
        realm,
        "Base",
        1,
        &prototype,
        parent,
        |realm, _, target, args| {
            let this = create_this(realm, &target)?;
            this.define(
                Key::name("count"),
                Property::data(Value::Number(args.len() as f64)),
            );
            Ok(this)
        },
    )
}

/// A class extending `parent` whose construction is `body`.
fn derived(
    realm: &mut Realm,
    parent: &Object,
    fields: Option<Object>,
    body: impl Fn(&mut Realm, &Frame, &[Value]) -> Result<Object, Thrown> + 'static,
) -> Object {
    let (prototype_parent, constructor_parent) =
        heritage(realm, Some(&Value::Object(parent.clone()))).unwrap();
    let prototype = Object::new(prototype_parent, Kind::Ordinary);
    create(
        realm,
        "Derived",
        0,
        &prototype,
        constructor_parent,
        move |realm, function, target, args| {
            let frame = frame(function.clone(), target, fields.clone());
            body(realm, &frame, args)
        },
    )
}

#[test]
fn class_constructors_reject_calls_and_construct_with_new_target() {
    let mut realm = Realm::new();
    let class = Value::Object(base(&mut realm));
    let called = ops::call(&mut realm, &class, Value::Undefined, &[]);
    assert_eq!(
        message(&mut realm, called),
        "TypeError: Class constructor Base cannot be invoked without 'new'"
    );
    let instance = ops::construct(&mut realm, &class, &[Value::Null]).unwrap();
    same!(get(&mut realm, &instance, "count"), Value::Number(1.0));
    let prototype = get(&mut realm, &class, "prototype");
    assert_eq!(
        instance.as_object().unwrap().data().prototype,
        prototype.as_object().cloned()
    );
    same!(get(&mut realm, &prototype, "constructor"), class);
    let descriptor = class
        .as_object()
        .unwrap()
        .own_property(&Key::name("prototype"))
        .unwrap();
    assert!(!descriptor.configurable && !descriptor.enumerable);
    // A new target without an object prototype falls back to Object.prototype.
    let target = realm.function("target", 0, true, |_, _, _| Ok(Value::Undefined));
    target.define(Key::name("prototype"), Property::data(Value::Null));
    let object = create_this(&mut realm, &target).unwrap();
    assert_eq!(
        object.data().prototype,
        Some(realm.object_prototype.clone())
    );
}

#[test]
fn heritage_validates_constructors_and_prototypes() {
    let mut realm = Realm::new();
    let (prototype, parent) = heritage(&mut realm, None).unwrap();
    assert_eq!(prototype, Some(realm.object_prototype.clone()));
    assert_eq!(parent, realm.function_prototype);
    let (prototype, parent) = heritage(&mut realm, Some(&Value::Null)).unwrap();
    assert_eq!(prototype, None);
    assert_eq!(parent, realm.function_prototype);
    let class = base(&mut realm);
    let (prototype, parent) = heritage(&mut realm, Some(&Value::Object(class.clone()))).unwrap();
    assert!(prototype.is_some());
    assert_eq!(parent, class);
    let nulled = realm.function("nulled", 0, true, |_, _, _| Ok(Value::Undefined));
    nulled.define(Key::name("prototype"), Property::data(Value::Null));
    let (prototype, parent) = heritage(&mut realm, Some(&Value::Object(nulled.clone()))).unwrap();
    assert_eq!((prototype, parent), (None, nulled.clone()));
    nulled.define(Key::name("prototype"), Property::data(Value::Number(1.0)));
    let invalid = heritage(&mut realm, Some(&Value::Object(nulled)));
    assert_eq!(
        message(&mut realm, invalid),
        "TypeError: Class extends value does not have valid prototype property"
    );
    let plain = realm.function("plain", 0, false, |_, _, _| Ok(Value::Undefined));
    for value in [Value::Object(plain), Value::Number(1.0)] {
        let refused = heritage(&mut realm, Some(&value));
        assert_eq!(
            message(&mut realm, refused),
            "TypeError: Class extends value is not a constructor or null"
        );
    }
    let failing = realm.function("failing", 0, true, |_, _, _| Ok(Value::Undefined));
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "prototype read"))
    });
    failing.define(
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
    let thrown = heritage(&mut realm, Some(&Value::Object(failing)));
    assert_eq!(message(&mut realm, thrown), "RangeError: prototype read");
}

#[test]
fn derived_constructors_bind_this_once_through_super() {
    let mut realm = Realm::new();
    let parent = base(&mut realm);
    let fields = realm.function("", 0, false, |realm, this, _| {
        ops::set(realm, &this, &Key::name("field"), Value::Boolean(true))?;
        Ok(Value::Undefined)
    });
    let class = derived(&mut realm, &parent, Some(fields), |realm, frame, args| {
        let early = this_value(realm, frame);
        assert!(early.is_err());
        let this = super_call(realm, frame, args)?;
        same!(this_value(realm, frame)?, this);
        let again = super_call(realm, frame, args);
        assert_eq!(
            message(realm, again),
            "ReferenceError: Super constructor may only be called once"
        );
        finish(realm, frame, Value::Undefined)
    });
    let instance = ops::construct(
        &mut realm,
        &Value::Object(class),
        &[Value::Null, Value::Null],
    )
    .unwrap();
    same!(get(&mut realm, &instance, "count"), Value::Number(2.0));
    same!(get(&mut realm, &instance, "field"), Value::Boolean(true));

    let defaulted = derived(&mut realm, &parent, None, default_derived);
    let instance = ops::construct(&mut realm, &Value::Object(defaulted.clone()), &[]).unwrap();
    same!(get(&mut realm, &instance, "count"), Value::Number(0.0));
    let (prototype, _) = heritage(&mut realm, Some(&Value::Object(defaulted))).unwrap();
    assert_eq!(instance.as_object().unwrap().data().prototype, prototype);

    let failing_fields = realm.function("", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "field"))
    });
    let failing = derived(&mut realm, &parent, Some(failing_fields), default_derived);
    let thrown = ops::construct(&mut realm, &Value::Object(failing), &[]);
    assert_eq!(message(&mut realm, thrown), "RangeError: field");
    let throwing_parent = {
        let prototype = realm.object();
        create(
            &realm,
            "Thrower",
            0,
            &prototype,
            realm.function_prototype.clone(),
            |realm, _, _, _| Err(realm.throw(ErrorKind::Range, "parent")),
        )
    };
    let child = derived(&mut realm, &throwing_parent, None, default_derived);
    let thrown = ops::construct(&mut realm, &Value::Object(child), &[]);
    assert_eq!(message(&mut realm, thrown), "RangeError: parent");
}

#[test]
fn derived_constructor_results_are_objects_or_initialized_this() {
    let mut realm = Realm::new();
    let parent = base(&mut realm);
    let replaced = realm.object();
    let returned = replaced.clone();
    let class = derived(&mut realm, &parent, None, move |realm, frame, _| {
        finish(realm, frame, Value::Object(returned.clone()))
    });
    let instance = ops::construct(&mut realm, &Value::Object(class), &[]).unwrap();
    same!(instance, Value::Object(replaced));
    let class = derived(&mut realm, &parent, None, |realm, frame, _| {
        finish(realm, frame, Value::Undefined)
    });
    let missing = ops::construct(&mut realm, &Value::Object(class), &[]);
    assert_eq!(
        message(&mut realm, missing),
        "ReferenceError: Must call super constructor in derived class before accessing 'this' or returning from derived constructor"
    );
    let class = derived(&mut realm, &parent, None, |realm, frame, _| {
        super_call(realm, frame, &[])?;
        finish(realm, frame, Value::Number(1.0))
    });
    let primitive = ops::construct(&mut realm, &Value::Object(class), &[]);
    assert_eq!(
        message(&mut realm, primitive),
        "TypeError: Derived constructors may only return object or undefined"
    );
    // A derived class whose parent was replaced by a non-constructor.
    let class = derived(&mut realm, &parent, None, default_derived);
    class.data_mut().prototype = None;
    let orphan = ops::construct(&mut realm, &Value::Object(class.clone()), &[]);
    assert_eq!(
        message(&mut realm, orphan),
        "TypeError: Super constructor is not a constructor"
    );
    class.data_mut().prototype = Some(realm.function_prototype.clone());
    let plain = ops::construct(&mut realm, &Value::Object(class), &[]);
    assert_eq!(
        message(&mut realm, plain),
        "TypeError: Super constructor is not a constructor"
    );
}

#[test]
fn base_constructors_initialize_fields_and_keep_returned_objects() {
    let mut realm = Realm::new();
    let (_, parent) = heritage(&mut realm, None).unwrap();
    let prototype = realm.object();
    let fields = realm.function("", 0, false, |realm, this, _| {
        ops::set(realm, &this, &Key::name("field"), Value::Number(7.0))?;
        Ok(Value::Undefined)
    });
    let class = create(
        &realm,
        "Plain",
        0,
        &prototype,
        parent,
        move |realm, _, target, _| default_base(realm, &target, Some(&fields)),
    );
    let instance = ops::construct(&mut realm, &Value::Object(class), &[]).unwrap();
    same!(get(&mut realm, &instance, "field"), Value::Number(7.0));
    let this = Value::Object(realm.object());
    let other = realm.object();
    assert_eq!(finish_base(&this, Value::Object(other.clone())), other);
    same!(Value::Object(finish_base(&this, Value::Number(1.0))), this);
    let not_callable = realm.object();
    let thrown = initialize(&mut realm, Some(&not_callable), &this);
    assert!(thrown.is_err());
    initialize(&mut realm, None, &this).unwrap();
    let target = realm.function("target", 0, true, |_, _, _| Ok(Value::Undefined));
    let failing = realm.function("", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "init"))
    });
    let thrown = default_base(&mut realm, &target, Some(&failing));
    assert_eq!(message(&mut realm, thrown), "RangeError: init");
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "prototype"))
    });
    target.define(
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
    let thrown = default_base(&mut realm, &target, None);
    assert_eq!(message(&mut realm, thrown), "RangeError: prototype");
}

#[test]
fn methods_accessors_and_fields_define_class_properties() {
    let mut realm = Realm::new();
    let target = realm.object();
    let function = Value::Object(realm.function("m", 0, false, |_, _, _| Ok(Value::Undefined)));
    define_method(&target, Key::name("m"), function.clone());
    let method = target.own_property(&Key::name("m")).unwrap();
    assert!(!method.enumerable && method.configurable);
    define_accessor(&target, Key::name("a"), Some(function.clone()), None);
    define_accessor(&target, Key::name("a"), None, Some(function.clone()));
    let Slot::Accessor { get, set } = target.own_property(&Key::name("a")).unwrap().slot else {
        panic!("accessor");
    };
    assert!(get.is_some() && set.is_some());
    define_accessor(&target, Key::name("m"), None, Some(function.clone()));
    let Slot::Accessor { get, set } = target.own_property(&Key::name("m")).unwrap().slot else {
        panic!("accessor");
    };
    assert!(get.is_none() && set.is_some());
    let instance = Value::Object(realm.object());
    define_field(&realm, &instance, Key::name("f"), Value::Number(1.0)).unwrap();
    let field = instance
        .as_object()
        .unwrap()
        .own_property(&Key::name("f"))
        .unwrap();
    assert!(field.enumerable && field.configurable);
    instance.as_object().unwrap().prevent_extensions();
    let refused = define_field(&realm, &instance, Key::name("g"), Value::Null);
    assert_eq!(
        message(&mut realm, refused),
        "TypeError: Cannot define property g"
    );
    let method = private_method(&function);
    assert!(matches!(
        method.slot,
        Slot::Data {
            writable: false,
            ..
        }
    ));
    let accessor = private_accessor(Some(&function), None);
    assert!(matches!(
        accessor.slot,
        Slot::Accessor {
            get: Some(_),
            set: None
        }
    ));
}

#[test]
fn super_property_reads_start_above_the_home_object() {
    let mut realm = Realm::new();
    let grandparent = realm.object();
    grandparent.define(Key::name("deep"), Property::data(Value::str("deep")));
    let getter = realm.function("get", 0, false, |realm, this, _| {
        ops::get(realm, &this, &Key::name("name"))
    });
    grandparent.define(
        Key::name("getter"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    grandparent.define(
        Key::name("setterOnly"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let parent = Object::new(Some(grandparent), Kind::Ordinary);
    parent.define(Key::name("near"), Property::data(Value::str("near")));
    let home = Object::new(Some(parent), Kind::Ordinary);
    home.define(Key::name("near"), Property::data(Value::str("own")));
    let receiver = Value::Object(realm.object());
    ops::set(
        &mut realm,
        &receiver,
        &Key::name("name"),
        Value::str("receiver"),
    )
    .unwrap();
    let mut read =
        |key: &str| text(&super_get(&mut realm, &home, &receiver, &Key::name(key)).unwrap());
    assert_eq!(read("near"), "near");
    assert_eq!(read("deep"), "deep");
    assert_eq!(read("getter"), "receiver");
    assert_eq!(read("setterOnly"), "Undefined");
    assert_eq!(read("missing"), "Undefined");
}

#[test]
fn private_elements_are_invisible_and_checked() {
    let mut realm = Realm::new();
    let name = Symbol::new(Some(JsStr::from("#x")));
    let object = Value::Object(realm.object());
    let read = private_get(&mut realm, &object, &name);
    assert_eq!(
        message(&mut realm, read),
        "TypeError: Cannot read private member #x from an object whose class did not declare it"
    );
    let write = private_set(&mut realm, &Value::Number(1.0), &name, Value::Null);
    assert_eq!(
        message(&mut realm, write),
        "TypeError: Cannot write private member #x from an object whose class did not declare it"
    );
    private_define(&realm, &object, &name, Property::data(Value::Number(1.0))).unwrap();
    assert!(object.as_object().unwrap().own_keys().is_empty());
    let twice = private_define(&realm, &object, &name, Property::data(Value::Null));
    assert_eq!(
        message(&mut realm, twice),
        "TypeError: Cannot initialize a private member twice on the same object"
    );
    private_set(&mut realm, &object, &name, Value::Number(2.0)).unwrap();
    same!(
        private_get(&mut realm, &object, &name).unwrap(),
        Value::Number(2.0)
    );
    assert!(private_in(&realm, &name, &object).unwrap());
    assert!(!private_in(&realm, &Symbol::new(None), &object).unwrap());
    let primitive = private_in(&realm, &name, &Value::Null);
    assert_eq!(
        message(&mut realm, primitive),
        "TypeError: Cannot use 'in' operator to search for a private member in a primitive"
    );
    let unnamed = Symbol::new(None);
    let read = private_get(&mut realm, &object, &unnamed);
    assert_eq!(
        message(&mut realm, read),
        "TypeError: Cannot read private member  from an object whose class did not declare it"
    );

    let method = Symbol::new(Some(JsStr::from("#m")));
    let function = Value::Object(realm.function("m", 0, false, |_, _, _| Ok(Value::Undefined)));
    private_define(&realm, &object, &method, private_method(&function)).unwrap();
    same!(private_get(&mut realm, &object, &method).unwrap(), function);
    let refused = private_set(&mut realm, &object, &method, Value::Null);
    assert_eq!(
        message(&mut realm, refused),
        "TypeError: Private method is not writable"
    );

    let getter = Value::Object(realm.function("get", 0, false, |realm, this, _| {
        ops::get(realm, &this, &Key::name("stored"))
    }));
    let setter = Value::Object(realm.function("set", 1, false, |realm, this, args| {
        ops::set(realm, &this, &Key::name("stored"), args[0].clone())?;
        Ok(Value::Undefined)
    }));
    let both = Symbol::new(Some(JsStr::from("#both")));
    private_define(
        &realm,
        &object,
        &both,
        private_accessor(Some(&getter), Some(&setter)),
    )
    .unwrap();
    private_set(&mut realm, &object, &both, Value::Number(5.0)).unwrap();
    same!(
        private_get(&mut realm, &object, &both).unwrap(),
        Value::Number(5.0)
    );
    let failing = Value::Object(realm.function("set", 1, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "setter"))
    }));
    let throwing = Symbol::new(Some(JsStr::from("#throwing")));
    private_define(
        &realm,
        &object,
        &throwing,
        private_accessor(None, Some(&failing)),
    )
    .unwrap();
    let thrown = private_set(&mut realm, &object, &throwing, Value::Null);
    assert_eq!(message(&mut realm, thrown), "RangeError: setter");
    let read = private_get(&mut realm, &object, &throwing);
    assert_eq!(
        message(&mut realm, read),
        "TypeError: Private accessor was defined without a getter"
    );
    let only = Symbol::new(Some(JsStr::from("#only")));
    private_define(
        &realm,
        &object,
        &only,
        private_accessor(Some(&getter), None),
    )
    .unwrap();
    let write = private_set(&mut realm, &object, &only, Value::Null);
    assert_eq!(
        message(&mut realm, write),
        "TypeError: Private accessor was defined without a setter"
    );
}

#[test]
fn weak_object_handles_do_not_keep_objects_alive() {
    let object = Object::new(None, Kind::Ordinary);
    let weak = object.downgrade();
    assert_eq!(weak.clone().upgrade(), Some(object.clone()));
    drop(object);
    assert!(weak.upgrade().is_none());
}

#[test]
fn private_method_and_accessor_brand_errors_name_the_lexical_class() {
    let mut realm = Realm::new();
    let key = Symbol::class_member("A", "#m");
    let weak = key.downgrade();
    let object = Value::Object(realm.object());
    let read = private_get(&mut realm, &object, &key);
    assert_eq!(
        message(&mut realm, read),
        "TypeError: Receiver must be an instance of class A"
    );
    let write = private_set(&mut realm, &Value::Number(0.0), &key, Value::Null);
    assert_eq!(
        message(&mut realm, write.map(|()| Value::Undefined)),
        "TypeError: Receiver must be an instance of class A"
    );
    assert!(weak.upgrade().unwrap() == key);
    drop(key);
    assert!(weak.upgrade().is_none());
}
