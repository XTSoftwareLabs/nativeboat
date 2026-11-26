use super::*;
use crate::ops;

fn message(realm: &mut Realm, thrown: &Thrown) -> String {
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

#[test]
fn realms_link_intrinsic_prototypes() {
    let realm = Realm::default();
    let prototype = |object: &Object| object.data().prototype.clone();
    assert_eq!(
        prototype(&realm.object()),
        Some(realm.object_prototype.clone())
    );
    assert_eq!(prototype(&realm.object_prototype), None);
    assert_eq!(
        prototype(&realm.array(vec![Value::Null])),
        Some(realm.array_prototype.clone())
    );
    assert_eq!(
        prototype(&realm.type_error_prototype),
        Some(realm.error_prototype.clone())
    );
    // Function.prototype is itself callable and returns undefined.
    let mut realm = realm;
    let callable = Value::Object(realm.function_prototype.clone());
    assert!(matches!(
        ops::call(&mut realm, &callable, Value::Null, &[]),
        Ok(Value::Undefined)
    ));
    assert!(format!("{realm:?}").starts_with("Realm"));
    realm.set_dynamic_import_loader(|_, _| Ok(Value::Undefined));
    assert!(format!("{realm:?}").contains("DynamicImportLoader"));
    realm.set_global("answer", Value::Number(42.0));
    assert!(realm.global.own_property(&Key::name("answer")).is_some());
}

#[test]
fn mapped_arguments_alias_parameters_until_redefinition_or_deletion() {
    use crate::{Descriptor, binding};

    let mut realm = Realm::new();
    let parameter = binding::new(Some(Value::Number(1.0)));
    let arguments = realm.mapped_arguments(
        &[Value::Number(1.0)],
        &[(0, parameter.clone()), (7, parameter.clone())],
    );
    let value = Value::Object(arguments.clone());
    binding::write(&realm, &parameter, "parameter", Value::Number(2.0)).unwrap();
    assert!(matches!(
        ops::get(&mut realm, &value, &Key::Index(0)).unwrap(),
        Value::Number(2.0)
    ));
    ops::set(&mut realm, &value, &Key::Index(0), Value::Number(3.0)).unwrap();
    assert!(matches!(
        binding::read(&realm, &parameter, "parameter").unwrap(),
        Value::Number(3.0)
    ));
    assert!(arguments.define_own_property(
        &Key::Index(0),
        &Descriptor {
            enumerable: Some(false),
            ..Descriptor::default()
        }
    ));
    assert!(arguments.define_own_property(
        &Key::Index(0),
        &Descriptor {
            value: Some(Value::Number(4.0)),
            writable: Some(false),
            ..Descriptor::default()
        }
    ));
    assert!(matches!(
        binding::read(&realm, &parameter, "parameter").unwrap(),
        Value::Number(4.0)
    ));
    binding::write(&realm, &parameter, "parameter", Value::Number(5.0)).unwrap();
    assert!(matches!(
        ops::get(&mut realm, &value, &Key::Index(0)).unwrap(),
        Value::Number(4.0)
    ));

    let arguments = realm.mapped_arguments(&[Value::Number(5.0)], &[(0, parameter.clone())]);
    let getter = realm.function("get", 0, false, |_, _, _| Ok(Value::Number(9.0)));
    assert!(arguments.define_own_property(
        &Key::Index(0),
        &Descriptor {
            get: Some(Some(getter)),
            ..Descriptor::default()
        }
    ));
    binding::write(&realm, &parameter, "parameter", Value::Number(6.0)).unwrap();
    assert!(matches!(
        ops::get(&mut realm, &Value::Object(arguments), &Key::Index(0)).unwrap(),
        Value::Number(9.0)
    ));
    let arguments = realm.mapped_arguments(&[Value::Number(6.0)], &[(0, parameter.clone())]);
    assert!(arguments.delete(&Key::Index(0)));
    binding::write(&realm, &parameter, "parameter", Value::Number(7.0)).unwrap();
    assert!(arguments.own_property(&Key::Index(0)).is_none());
}

#[test]
fn strict_arguments_have_ordinary_indexes_restricted_callee_and_original_iterator() {
    let mut realm = Realm::new();
    let original_values = realm.array_values.clone();
    realm
        .array_prototype
        .define(Key::name("values"), Property::data(Value::Null));
    let object = realm.arguments(&[Value::Number(1.0), Value::str("two")]);
    let value = Value::Object(object.clone());
    assert_eq!(
        object.data().prototype,
        Some(realm.object_prototype.clone())
    );
    assert!(!object.is_array());
    assert!(format!("{:?}", object.data().kind).contains("Arguments"));
    let index = object.own_property(&Key::Index(0)).unwrap();
    assert!(index.enumerable && index.configurable);
    assert!(matches!(index.slot, Slot::Data { writable: true, .. }));
    let length = object.own_property(&Key::name("length")).unwrap();
    assert!(!length.enumerable && length.configurable);
    assert!(matches!(
        length.slot,
        Slot::Data {
            writable: true,
            value: Value::Number(2.0)
        }
    ));
    let iterator_key = Key::Symbol(realm.symbols.iterator.clone());
    let iterator = ops::get(&mut realm, &value, &iterator_key).unwrap();
    assert_eq!(iterator.as_object(), Some(&original_values));
    let tagger = ops::get(&mut realm, &value, &Key::name("toString")).unwrap();
    let tag = ops::call(&mut realm, &tagger, value.clone(), &[]).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &tag).unwrap().to_rust_lossy(),
        "[object Arguments]"
    );
    let callee = object.own_property(&Key::name("callee")).unwrap();
    assert!(!callee.enumerable && !callee.configurable);
    let Slot::Accessor { get, set } = callee.slot else {
        panic!("restricted accessor")
    };
    assert_eq!(get, Some(realm.throw_type_error.clone()));
    assert_eq!(set, Some(realm.throw_type_error.clone()));
    assert!(!realm.throw_type_error.data().extensible);
    for name in ["length", "name"] {
        let property = realm
            .throw_type_error
            .own_property(&Key::name(name))
            .unwrap();
        assert!(!property.enumerable && !property.configurable);
        assert!(matches!(
            property.slot,
            Slot::Data {
                writable: false,
                ..
            }
        ));
    }
    assert!(ops::get(&mut realm, &value, &Key::name("callee")).is_err());
    assert!(ops::set(&mut realm, &value, &Key::name("callee"), Value::Null).is_err());
    assert!(ops::delete(&mut realm, &value, &Key::name("callee")).is_err());
    assert!(matches!(
        ops::get(&mut realm, &value, &Key::name("caller")).unwrap(),
        Value::Undefined
    ));
    ops::set(&mut realm, &value, &Key::Index(0), Value::Number(9.0)).unwrap();
    assert!(ops::delete(&mut realm, &value, &Key::Index(1)).unwrap());
    let copied = realm.arguments(&[]);
    let other = copied.own_property(&Key::name("callee")).unwrap();
    let Slot::Accessor { get, set } = other.slot else {
        panic!("restricted accessor")
    };
    assert_eq!(get, Some(realm.throw_type_error.clone()));
    assert_eq!(set, Some(realm.throw_type_error.clone()));
}

#[test]
fn sparse_arrays_preserve_holes_and_iteration_values() {
    let mut realm = Realm::new();
    let array = realm.sparse_array(vec![None, Some(Value::Number(2.0)), None]);
    assert_eq!(array.data().prototype, Some(realm.array_prototype.clone()));
    assert!(array.own_property(&Key::Index(0)).is_none());
    assert!(array.own_property(&Key::Index(1)).is_some());
    let value = Value::Object(array);
    assert!(ops::strict_equals(
        &ops::get(&mut realm, &value, &Key::name("length")).unwrap(),
        &Value::Number(3.0)
    ));
    let values = crate::iter::collect(&mut realm, &value).unwrap();
    assert_eq!(values.len(), 3);
    assert!(matches!(values[0], Value::Undefined));
    assert!(ops::strict_equals(&values[1], &Value::Number(2.0)));
    assert!(matches!(values[2], Value::Undefined));
}

#[test]
fn functions_have_names_lengths_and_constructor_prototypes() {
    let mut realm = Realm::new();
    let function = realm.function("make", 2, true, |_, this, _| Ok(this));
    let value = Value::Object(function.clone());
    let name = ops::get(&mut realm, &value, &Key::name("name")).unwrap();
    assert!(ops::strict_equals(&name, &Value::str("make")));
    let length = ops::get(&mut realm, &value, &Key::name("length")).unwrap();
    assert!(ops::strict_equals(&length, &Value::Number(2.0)));
    let prototype = ops::get(&mut realm, &value, &Key::name("prototype")).unwrap();
    let constructor = ops::get(&mut realm, &prototype, &Key::name("constructor")).unwrap();
    assert!(ops::strict_equals(&constructor, &value));
    let arrow = realm.function("", 0, false, |_, _, _| Ok(Value::Undefined));
    assert!(arrow.own_property(&Key::name("prototype")).is_none());
}

#[test]
fn errors_carry_kind_and_message() {
    let mut realm = Realm::new();
    for (kind, text) in [
        (ErrorKind::Error, "Error: plain"),
        (ErrorKind::Type, "TypeError: plain"),
        (ErrorKind::Range, "RangeError: plain"),
        (ErrorKind::Reference, "ReferenceError: plain"),
    ] {
        let thrown = realm.throw(kind, "plain");
        assert_eq!(message(&mut realm, &thrown), text);
    }
}

#[test]
fn deep_recursion_becomes_a_range_error() {
    let mut realm = Realm::new();
    realm.max_depth = 50;
    let recurse: NativeCode = Rc::new(|realm, this, args| {
        let code = this.as_object().unwrap().code().unwrap().0;
        realm.enter(&code, this, args)
    });
    let function = Object::new(
        None,
        Kind::Function {
            code: recurse.clone(),
            constructor: false,
            realm: 0,
        },
    );
    let thrown = realm
        .enter(&recurse, Value::Object(function), &[])
        .unwrap_err();
    assert_eq!(
        message(&mut realm, &thrown),
        "RangeError: Maximum call stack size exceeded"
    );
    // The depth unwinds, so later calls succeed.
    let ok: NativeCode = Rc::new(|_, _, _| Ok(Value::Null));
    assert!(matches!(
        realm.enter(&ok, Value::Undefined, &[]),
        Ok(Value::Null)
    ));
}

#[test]
fn template_objects_are_cached_per_site_and_frozen() {
    let mut realm = Realm::new();
    let first = realm.template_object(1, &[Some(&[97][..]), None], &[&[97][..], &[92, 117][..]]);
    let again = realm.template_object(1, &[], &[]);
    assert_eq!(first, again);
    let other = realm.template_object(2, &[Some(&[][..])], &[&[][..]]);
    assert_ne!(first, other);
    assert!(!first.is_extensible());
    let element = first.own_property(&Key::Index(0)).unwrap();
    assert!(!element.configurable);
    assert!(matches!(
        element.slot,
        Slot::Data {
            writable: false,
            ..
        }
    ));
    assert!(matches!(
        first.own_property(&Key::Index(1)).unwrap().slot,
        Slot::Data {
            value: Value::Undefined,
            ..
        }
    ));
    let raw = first.own_property(&Key::name("raw")).unwrap();
    assert!(!raw.enumerable && !raw.configurable);
    let Slot::Data {
        value: Value::Object(raw),
        writable: false,
    } = raw.slot
    else {
        panic!("raw strings array");
    };
    assert!(!raw.is_extensible());
    let length = first.own_property(&Key::name("length")).unwrap();
    assert!(matches!(
        length.slot,
        Slot::Data {
            writable: false,
            ..
        }
    ));
}

#[test]
fn uncaught_reports_show_converted_values_or_their_native_form() {
    let mut realm = Realm::new();
    let error = realm.throw(ErrorKind::Type, "boom").0;
    assert_eq!(
        realm.uncaught(&error),
        "Uncaught TypeError: boom
"
    );
    assert_eq!(
        realm.uncaught(&Value::Number(3.0)),
        "Uncaught 3
"
    );
    // A symbol cannot be converted to a string.
    let symbol = Value::Symbol(realm.symbols.iterator.clone());
    assert!(realm.uncaught(&symbol).starts_with("Uncaught Symbol("));
}

#[test]
fn fixed_property_names_share_one_key() {
    let mut realm = Realm::new();
    let first = realm.name_key("length");
    let again = realm.name_key("length");
    match (&first, &again) {
        (Key::String(first), Key::String(again)) => {
            assert!(core::ptr::eq(first.units(), again.units()));
        }
        _ => panic!("names are string keys"),
    }
    assert_eq!(realm.name_key("7"), Key::Index(7));
    let object = realm.object();
    object.define(
        Key::name("length"),
        crate::Property::data(Value::Number(3.0)),
    );
    assert!(matches!(
        crate::ops::get_named(&mut realm, &Value::Object(object), "length"),
        Ok(Value::Number(3.0))
    ));
}

#[test]
fn page_script_checkpoints_settle_replies_across_realms() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut main = Realm::default();
    let mut preload = Realm::default();
    let mut page = Realm::default();
    let preload_address = &mut preload as *mut Realm as usize;
    let page_address = &mut page as *mut Realm as usize;
    // A reply chain: the main realm's job settles the preload's promise,
    // whose job settles the page's.
    let main_log = log.clone();
    main.enqueue(Box::new(move |_| {
        main_log.borrow_mut().push("main");
        let preload_log = main_log.clone();
        // SAFETY: the realms outlive the checkpoint below.
        let preload = unsafe { &mut *(preload_address as *mut Realm) };
        preload.enqueue(Box::new(move |_| {
            preload_log.borrow_mut().push("preload");
            let page_log = preload_log.clone();
            // SAFETY: as above.
            let page = unsafe { &mut *(page_address as *mut Realm) };
            page.enqueue(Box::new(move |_| {
                page_log.borrow_mut().push("page");
                Ok(())
            }));
            Ok(())
        }));
        Ok(())
    }));
    // Realms that run no page have no checkpoint.
    page.script_checkpoint().unwrap();
    assert!(log.borrow().is_empty());
    // SAFETY: the realms are distinct and outlive `page`'s use of them.
    unsafe {
        page.connect_electron_ipc(&mut main);
        page.run_page_scripts(&mut preload);
    }
    page.script_checkpoint().unwrap();
    assert_eq!(*log.borrow(), ["main", "preload", "page"]);
}

#[test]
fn preload_ticks_page_code_queues_wait_until_the_page_renders() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut preload = Realm::default();
    let mut page = Realm::default();
    // SAFETY: the realms are distinct and outlive `page`'s use of them.
    unsafe { page.run_page_scripts(&mut preload) };
    // Page code calls into the preload, whose promise job queues a tick.
    let job_log = log.clone();
    preload.enqueue(Box::new(move |realm| {
        job_log.borrow_mut().push("job");
        let tick_log = job_log.clone();
        realm.enqueue_tick(Box::new(move |_| {
            tick_log.borrow_mut().push("deferred");
            Ok(())
        }));
        Ok(())
    }));
    page.script_checkpoint().unwrap();
    assert_eq!(*log.borrow(), ["job"]);
    assert!(preload.has_deferred_ticks() && !preload.has_queued_jobs());
    // Its ticks run ahead of those queued since, after the page rendered.
    let later = log.clone();
    preload.enqueue_tick(Box::new(move |_| {
        later.borrow_mut().push("later");
        Ok(())
    }));
    preload.run_deferred_ticks();
    assert_eq!(*log.borrow(), ["job", "deferred", "later"]);
    assert!(!preload.has_deferred_ticks());
    preload.run_deferred_ticks();
    // A tick that throws reaches the host at the preload's next step.
    preload.enqueue(Box::new(|realm| {
        realm.enqueue_tick(Box::new(|realm| Err(realm.throw(ErrorKind::Error, "tick"))));
        Ok(())
    }));
    page.script_checkpoint().unwrap();
    preload.run_deferred_ticks();
    assert_eq!(preload.host_errors.len(), 1);
}
