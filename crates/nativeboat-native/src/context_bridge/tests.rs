use super::*;
use alloc::rc::Rc;
use core::cell::RefCell;

fn get(realm: &mut Realm, value: &Value, name: &str) -> Value {
    ops::get(realm, value, &Key::name(name)).unwrap()
}

fn call(realm: &mut Realm, value: &Value, name: &str, args: &[Value]) -> crate::Completion {
    let function = get(realm, value, name);
    ops::call(realm, &function, Value::Undefined, args)
}

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

/// A preload realm exposing `api` and a page realm it is bridged to.
fn bridged() -> (Box<Realm>, Box<Realm>, Value) {
    let mut preload = Box::new(Realm::new());
    let page = Box::new(Realm::new());
    let api = preload.object();
    let add = preload.function("add", 2, false, |realm, _, args| {
        let left = ops::to_number(realm, &args[0])?;
        let right = ops::to_number(realm, &args[1])?;
        Ok(Value::Number(left + right))
    });
    let fail = preload.function("fail", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "bad"))
    });
    let invoke = preload.function("invoke", 1, false, |realm, _, args| {
        let detail = realm.object();
        detail.define(Key::name("from"), Property::data(Value::str("preload")));
        let result = ops::call(realm, &args[0], Value::Undefined, &[Value::Object(detail)])?;
        Ok(Value::Object(realm.array(vec![result, args[0].clone()])))
    });
    let later = preload.function("later", 1, false, |realm, _, args| {
        let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
        let settle = if ops::to_boolean(&args[0]) {
            resolve
        } else {
            reject
        };
        let value = realm.object();
        value.define(Key::name("v"), Property::data(Value::Number(1.0)));
        ops::call(realm, &settle, Value::Undefined, &[Value::Object(value)])?;
        Ok(promise)
    });
    let symbol = preload.function("symbol", 0, false, |_, _, _| {
        Ok(Value::Symbol(crate::Symbol::new(None)))
    });
    let nested = preload.object();
    nested.define(Key::name("x"), Property::data(Value::Number(1.0)));
    nested.define(Key::name("hidden"), Property::hidden(Value::Number(2.0)));
    nested.define(
        Key::Symbol(crate::Symbol::new(None)),
        Property::data(Value::Null),
    );
    nested.define(
        Key::name("self"),
        Property::data(Value::Object(nested.clone())),
    );
    let date = Object::new(Some(preload.date_prototype.clone()), Kind::Date(5.0));
    let bytes = crate::typed_array_builtin::new_bytes(&preload, vec![1, 2, 3], None);
    let list = preload.array(vec![Value::Number(1.0), Value::str("two")]);
    let error = preload.error(ErrorKind::Type, "copied");
    // A map holding the list, and a set of two numbers.
    let collection = |preload: &mut Realm, name: &str, method: &str, entries: &[&[Value]]| {
        let global = Value::Object(preload.global.clone());
        let constructor = get(preload, &global, name);
        let collection = ops::construct(preload, &constructor, &[]).unwrap();
        let add = get(preload, &collection, method);
        for entry in entries {
            ops::call(preload, &add, collection.clone(), entry).unwrap();
        }
        collection.as_object().unwrap().clone()
    };
    let map = collection(
        &mut preload,
        "Map",
        "set",
        &[&[Value::str("k"), Value::Object(list.clone())]],
    );
    let set = collection(
        &mut preload,
        "Set",
        "add",
        &[&[Value::Number(1.0)], &[Value::Number(2.0)]],
    );
    for (name, value) in [
        ("add", add),
        ("fail", fail),
        ("invoke", invoke),
        ("later", later),
        ("symbol", symbol),
        ("nested", nested),
        ("date", date),
        ("bytes", bytes),
        ("list", list),
        ("error", error),
        ("map", map),
        ("set", set),
    ] {
        api.define(Key::name(name), Property::data(Value::Object(value)));
    }
    preload
        .electron_bridge_exports
        .push(("api".into(), Value::Object(api)));
    preload
        .electron_bridge_exports
        .push(("bad".into(), Value::Symbol(crate::Symbol::new(None))));
    (preload, page, Value::Undefined)
}

#[test]
fn exposed_values_are_copied_into_the_page() {
    let (mut preload, mut page, _) = bridged();
    // SAFETY: both realms are boxed and live for the test.
    unsafe { expose(&mut *preload, &mut *page) };
    let global = Value::Object(page.global.clone());
    let api = get(&mut page, &global, "api");
    assert!(matches!(get(&mut page, &global, "bad"), Value::Undefined));
    let api_object = api.as_object().unwrap();
    assert_eq!(
        api_object.data().prototype.as_ref(),
        Some(&page.object_prototype)
    );
    let sum = call(
        &mut page,
        &api,
        "add",
        &[Value::Number(2.0), Value::Number(3.0)],
    )
    .unwrap();
    assert!(matches!(sum, Value::Number(5.0)));
    let nested = get(&mut page, &api, "nested");
    assert!(matches!(get(&mut page, &nested, "x"), Value::Number(1.0)));
    assert!(matches!(
        get(&mut page, &nested, "hidden"),
        Value::Undefined
    ));
    let cycle = get(&mut page, &nested, "self");
    assert!(ops::strict_equals(&cycle, &nested));
    let date = get(&mut page, &api, "date");
    assert!(matches!(
        date.as_object().unwrap().data().kind,
        Kind::Date(5.0)
    ));
    let bytes = get(&mut page, &api, "bytes");
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&bytes).unwrap(),
        [1, 2, 3]
    );
    let list = get(&mut page, &api, "list");
    assert!(list.as_object().unwrap().is_array());
    assert_eq!(text(&mut page, &list), "1,two");
    let error = get(&mut page, &api, "error");
    let message = get(&mut page, &error, "message");
    assert_eq!(text(&mut page, &message), "copied");
    // Maps and sets arrive as the page's own, with copied entries.
    let map = get(&mut page, &api, "map");
    let page_map = get(&mut page, &global, "Map");
    let prototype = get(&mut page, &page_map, "prototype");
    assert_eq!(
        map.as_object().unwrap().data().prototype.as_ref(),
        prototype.as_object()
    );
    let function = get(&mut page, &map, "get");
    let entry = ops::call(&mut page, &function, map.clone(), &[Value::str("k")]).unwrap();
    assert!(entry.as_object().unwrap().is_array());
    assert_eq!(text(&mut page, &entry), "1,two");
    let set = get(&mut page, &api, "set");
    assert!(matches!(get(&mut page, &set, "size"), Value::Number(2.0)));
    let function = get(&mut page, &set, "has");
    let has = ops::call(&mut page, &function, set.clone(), &[Value::Number(2.0)]).unwrap();
    assert!(matches!(has, Value::Boolean(true)));
    let thrown = call(&mut page, &api, "fail", &[]).unwrap_err();
    let message = get(&mut page, &thrown.0, "message");
    assert_eq!(text(&mut page, &message), "bad");
    assert!(call(&mut page, &api, "symbol", &[]).is_err());
    let symbol = Value::Symbol(crate::Symbol::new(None));
    assert!(call(&mut page, &api, "add", &[symbol, Value::Null]).is_err());
}

#[test]
fn page_callbacks_run_in_the_page_with_one_proxy_each() {
    let (mut preload, mut page, _) = bridged();
    // SAFETY: both realms are boxed and live for the test.
    unsafe { expose(&mut *preload, &mut *page) };
    let global = Value::Object(page.global.clone());
    let api = get(&mut page, &global, "api");
    let page_global = page.global.clone();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let observed = seen.clone();
    let callback = page.function("callback", 1, false, move |realm, _, args| {
        // The callback runs in the page realm: its globals are the page's.
        observed.borrow_mut().push(realm.global == page_global);
        let from = ops::get(realm, &args[0], &Key::name("from"))?;
        Ok(from)
    });
    let callback = Value::Object(callback);
    let first = call(&mut page, &api, "invoke", core::slice::from_ref(&callback)).unwrap();
    let second = call(&mut page, &api, "invoke", core::slice::from_ref(&callback)).unwrap();
    assert_eq!(text(&mut page, &first), text(&mut page, &second));
    assert_eq!(*seen.borrow(), [true, true]);
    // The preload saw the same proxy for the same page function both times.
    assert_eq!(preload.bridged_functions.len(), 1);
    let throwing = page.function("throwing", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "from page"))
    });
    let thrown = call(&mut page, &api, "invoke", &[Value::Object(throwing)]).unwrap_err();
    let message = get(&mut page, &thrown.0, "message");
    assert_eq!(text(&mut page, &message), "from page");
    let symbolic = page.function("symbolic", 0, false, |_, _, _| {
        Err(Thrown(Value::Symbol(crate::Symbol::new(None))))
    });
    assert!(call(&mut page, &api, "invoke", &[Value::Object(symbolic)]).is_err());
}

#[test]
fn promises_settle_in_the_page_with_copies() {
    let (mut preload, mut page, _) = bridged();
    // SAFETY: both realms are boxed and live for the test.
    unsafe { expose(&mut *preload, &mut *page) };
    let global = Value::Object(page.global.clone());
    let api = get(&mut page, &global, "api");
    let results = Rc::new(RefCell::new(Vec::new()));
    for fulfilled in [true, false] {
        let promise = call(&mut page, &api, "later", &[Value::Boolean(fulfilled)]).unwrap();
        let record = results.clone();
        let handler = page.function("handler", 1, false, move |realm, _, args| {
            let value = ops::get(realm, &args[0], &Key::name("v"))?;
            record
                .borrow_mut()
                .push((fulfilled, ops::to_number(realm, &value)?));
            Ok(Value::Undefined)
        });
        let then = get(&mut page, &promise, "then");
        let handler = Value::Object(handler);
        ops::call(&mut page, &then, promise, &[handler.clone(), handler]).unwrap();
    }
    preload.run_jobs().unwrap();
    page.run_jobs().unwrap();
    assert_eq!(*results.borrow(), [(true, 1.0), (false, 1.0)]);
    // A promise whose value cannot cross rejects in the page.
    let symbol_promise = {
        let (promise, resolve, _) = crate::promise_builtin::capability(&preload);
        let symbol = Value::Symbol(crate::Symbol::new(None));
        ops::call(&mut preload, &resolve, Value::Undefined, &[symbol]).unwrap();
        promise
    };
    // SAFETY: as above.
    let copied =
        unsafe { copy(&mut *preload, &mut *page, &symbol_promise, &mut Vec::new()) }.unwrap();
    let rejected = Rc::new(RefCell::new(false));
    let flag = rejected.clone();
    let on_rejected = page.function("rejected", 1, false, move |_, _, _| {
        *flag.borrow_mut() = true;
        Ok(Value::Undefined)
    });
    let then = get(&mut page, &copied, "then");
    ops::call(
        &mut page,
        &then,
        copied,
        &[Value::Undefined, Value::Object(on_rejected)],
    )
    .unwrap();
    preload.run_jobs().unwrap();
    page.run_jobs().unwrap();
    assert!(*rejected.borrow());
}
