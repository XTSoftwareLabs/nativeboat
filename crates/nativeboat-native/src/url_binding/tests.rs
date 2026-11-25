use super::*;
use crate::Completion;
use crate::ops;

fn component(view: &crate::typed_array_builtin::View, index: f64) -> u32 {
    let Value::Number(value) = view.get(index).unwrap() else {
        panic!("Uint32 component")
    };
    value as u32
}

fn call(realm: &mut Realm, binding: &Object, name: &str, args: &[Value]) -> Completion {
    let function = ops::get(realm, &Value::Object(binding.clone()), &Key::name(name)).unwrap();
    ops::call(realm, &function, Value::Undefined, args)
}

fn value(realm: &mut Realm, object: &Value, name: &str) -> Value {
    ops::get(realm, object, &Key::name(name)).unwrap()
}

#[test]
fn node_22_parse_reference_and_failed_parse_buffer_preservation() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    let buffer = value(&mut realm, &Value::Object(binding.clone()), "urlComponents");
    let view = typed_array_builtin::view_of(&buffer).unwrap();
    {
        let args = [
            Value::str("https://user:pass@Exämple.com:443/a/../b?x=1#f"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("https://user:pass@xn--exmple-cua.com/b?x=1#f")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 17, 36, 4294967295, 36, 38, 42, 2]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    {
        let args = [
            Value::str("../c?x#y"),
            Value::str("https://example.org/a/b"),
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("https://example.org/c?x#y")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 19, 4294967295, 19, 21, 23, 2]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    {
        let args = [
            Value::str("http://0x7f000001"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("http://127.0.0.1/")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 16, 4294967295, 16, 4294967295, 4294967295, 0]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    {
        let args = [
            Value::str("http://[2001:0db8:0:0:0:0:0:1]:80/a"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("http://[2001:db8::1]/a")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 20, 4294967295, 20, 4294967295, 4294967295, 0]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    {
        let args = [
            Value::str("file:///C:/a/../b"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("file:///C:/b")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    {
        let args = [
            Value::str("data:text/plain,你好#x"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("data:text/plain,%E4%BD%A0%E5%A5%BD#x")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 34, 1]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    {
        let args = [
            Value::str("custom://user:pw@host:123/path?q#h"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("custom://user:pw@host:123/path?q#h")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 13, 16, 21, 123, 25, 30, 32, 1]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    {
        let args = [
            Value::str("blob:https://example.com/id"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("blob:https://example.com/id")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 4294967295, 1]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    {
        let args = [
            Value::str("http://"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(&result, &Value::Undefined));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 4294967295, 1]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(false)
        ));
        let error = call(
            &mut realm,
            &binding,
            "parse",
            &[
                Value::str("http://"),
                Value::Undefined,
                Value::Boolean(true),
            ],
        )
        .unwrap_err()
        .0;
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "code"),
            &Value::str("ERR_INVALID_URL")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "message"),
            &Value::str("Invalid URL")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "input"),
            &Value::str("http://")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "base"),
            &Value::Undefined
        ));
    }
    {
        let args = [
            Value::str("http://[bad]"),
            Value::Undefined,
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(&result, &Value::Undefined));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 4294967295, 1]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(false)
        ));
        let error = call(
            &mut realm,
            &binding,
            "parse",
            &[
                Value::str("http://[bad]"),
                Value::Undefined,
                Value::Boolean(true),
            ],
        )
        .unwrap_err()
        .0;
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "code"),
            &Value::str("ERR_INVALID_URL")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "message"),
            &Value::str("Invalid URL")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "input"),
            &Value::str("http://[bad]")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "base"),
            &Value::Undefined
        ));
    }
    {
        let args = [Value::str("path"), Value::Undefined, Value::Boolean(false)];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(&result, &Value::Undefined));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 4294967295, 1]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(false)
        ));
        let error = call(
            &mut realm,
            &binding,
            "parse",
            &[Value::str("path"), Value::Undefined, Value::Boolean(true)],
        )
        .unwrap_err()
        .0;
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "code"),
            &Value::str("ERR_INVALID_URL")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "message"),
            &Value::str("Invalid URL")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "input"),
            &Value::str("path")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "base"),
            &Value::Undefined
        ));
    }
    {
        let args = [
            Value::str("https://good"),
            Value::str("bad base"),
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(&result, &Value::Undefined));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 4294967295, 1]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(false)
        ));
        let error = call(
            &mut realm,
            &binding,
            "parse",
            &[
                Value::str("https://good"),
                Value::str("bad base"),
                Value::Boolean(true),
            ],
        )
        .unwrap_err()
        .0;
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "code"),
            &Value::str("ERR_INVALID_URL")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "message"),
            &Value::str("Invalid URL")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "input"),
            &Value::str("https://good")
        ));
        assert!(crate::ops::strict_equals(
            &value(&mut realm, &error, "base"),
            &Value::str("bad base")
        ));
    }
    {
        let args = [
            Value::str("?query"),
            Value::str("file:///C:/a/b"),
            Value::Boolean(false),
        ];
        let result = call(&mut realm, &binding, "parse", &args).unwrap();
        assert!(crate::ops::strict_equals(
            &result,
            &Value::str("file:///C:/a/b?query")
        ));
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 14, 4294967295, 6]
        );
        assert!(crate::ops::strict_equals(
            &call(&mut realm, &binding, "canParse", &args).unwrap(),
            &Value::Boolean(true)
        ));
    }
    for name in ["parse", "canParse", "getOrigin"] {
        assert!(call(&mut realm, &binding, name, &[]).is_err());
    }
    for (input, origin) in [
        ("http://u:p@host:80/a", "http://host"),
        ("https://host/a", "https://host"),
        ("file:///C:/a", "null"),
        ("data:a", "null"),
        ("blob:https://example.com/id", "https://example.com"),
    ] {
        let result = call(&mut realm, &binding, "getOrigin", &[Value::str(input)]).unwrap();
        assert!(ops::strict_equals(&result, &Value::str(origin)));
    }
    let error = call(&mut realm, &binding, "getOrigin", &[Value::str("bad")])
        .unwrap_err()
        .0;
    assert!(matches!(
        value(&mut realm, &error, "input"),
        Value::Undefined
    ));
    assert!(ops::strict_equals(
        &value(&mut realm, &error, "code"),
        &Value::str("ERR_INVALID_URL")
    ));
}

#[test]
fn scheme_types_and_native_module_identity() {
    for (input, expected) in [("ws://host", 3), ("ftp://host", 4), ("wss://host", 5)] {
        assert_eq!(components(&Url::parse(input).unwrap())[8], expected);
    }
    let mut realm = Realm::new();
    let a = crate::modules::load(&mut realm, "binding/url").unwrap();
    let b = crate::modules::load(&mut realm, "binding/url").unwrap();
    assert!(ops::strict_equals(&a, &b));
    let mut other = Realm::new();
    let c = crate::modules::load(&mut other, "binding/url").unwrap();
    assert!(!ops::strict_equals(&a, &c));
}

#[test]
fn node_22_setters_domains_origins_and_file_paths() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    let buffer = value(&mut realm, &Value::Object(binding.clone()), "urlComponents");
    let view = typed_array_builtin::view_of(&buffer).unwrap();
    // The reference corpus runs setters after this last successful parse.
    call(
        &mut realm,
        &binding,
        "parse",
        &[Value::str("?query"), Value::str("file:///C:/a/b")],
    )
    .unwrap();
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(0.0),
                Value::str("http:"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("http://user:password@example.org:8443/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 0, 'http:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 11, 20, 32, 8443, 37, 41, 45, 0]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(0.0),
                Value::str("file:"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 0, 'file:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 11, 20, 32, 8443, 37, 41, 45, 0]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(0.0),
                Value::str("data:"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org:8443/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 0, 'data:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 8443, 38, 42, 46, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(1.0),
                Value::str("other:99"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@other:99/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 1, 'other:99']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 27, 99, 30, 34, 38, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(1.0),
                Value::str("other:bad"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@other:8443/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 1, 'other:bad']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 27, 8443, 32, 36, 40, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(1.0),
                Value::str("[::1]:42"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@[::1]:42/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 1, '[::1]:42']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 27, 42, 30, 34, 38, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(2.0),
                Value::str("other"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@other:8443/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 2, 'other']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 27, 8443, 32, 36, 40, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(2.0),
                Value::str("other:99"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 2, 'other:99']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 27, 8443, 32, 36, 40, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(3.0),
                Value::str("80"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org:80/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 3, '80']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 80, 36, 40, 44, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(3.0),
                Value::str("99junk"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org:99/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 3, '99junk']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 99, 36, 40, 44, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(3.0),
                Value::str("65536"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 3, '65536']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 99, 36, 40, 44, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(3.0),
                Value::str(""),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 3, '']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 4294967295, 33, 37, 41, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(4.0),
                Value::str("a@b"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://a%40b:password@example.org:8443/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 4, 'a@b']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 13, 22, 34, 8443, 39, 43, 47, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(5.0),
                Value::str("a:b"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:a%3Ab@example.org:8443/a/b?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 5, 'a:b']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 18, 30, 8443, 35, 39, 43, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(6.0),
                Value::str("/a b/../c"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org:8443/c?x=1#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 6, '/a b/../c']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 8443, 38, 40, 44, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(7.0),
                Value::str("?q=中 a"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org:8443/a/b?q=%E4%B8%AD%20a#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 7, '?q=\\u4e2d a']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 8443, 38, 42, 58, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(7.0),
                Value::str(""),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org:8443/a/b#hash")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 7, '']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 8443, 38, 4294967295, 42, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(8.0),
                Value::str("#中 a"),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org:8443/a/b?x=1#%E4%B8%AD%20a")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 8, '#\\u4e2d a']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 8443, 38, 42, 46, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(8.0),
                Value::str(""),
            ],
        );
        assert!(
            ops::strict_equals(
                &result.unwrap(),
                &Value::str("https://user:password@example.org:8443/a/b?x=1")
            ),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 8, '']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 12, 21, 33, 8443, 38, 42, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(9.0),
                Value::str("https://new/path"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("https://new/path")),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 9, 'https://new/path']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 11, 4294967295, 11, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://user:password@example.org:8443/a/b?x=1#hash"),
                Value::Number(9.0),
                Value::str("bad"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['https://user:password@example.org:8443/a/b?x=1#hash', 9, 'bad']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 11, 4294967295, 11, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(0.0),
                Value::str("http:"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['file:///C:/a/b', 0, 'http:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 11, 4294967295, 11, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(1.0),
                Value::str("host:2"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['file:///C:/a/b', 1, 'host:2']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 11, 4294967295, 11, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(2.0),
                Value::str("host"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("file://host/C:/a/b")),
            "reference update ['file:///C:/a/b', 2, 'host']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(3.0),
                Value::str("99"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['file:///C:/a/b', 3, '99']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(4.0),
                Value::str("name"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['file:///C:/a/b', 4, 'name']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(5.0),
                Value::str("pw"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['file:///C:/a/b', 5, 'pw']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(6.0),
                Value::str("/path"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("file:///path")),
            "reference update ['file:///C:/a/b', 6, '/path']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(7.0),
                Value::str("?x"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("file:///C:/a/b?x")),
            "reference update ['file:///C:/a/b', 7, '?x']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 14, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///C:/a/b"),
                Value::Number(8.0),
                Value::str("#y"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("file:///C:/a/b#y")),
            "reference update ['file:///C:/a/b', 8, '#y']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 14, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(0.0),
                Value::str("http:"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['data:text/plain,a', 0, 'http:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 14, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(1.0),
                Value::str("host:2"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['data:text/plain,a', 1, 'host:2']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 14, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(2.0),
                Value::str("host"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['data:text/plain,a', 2, 'host']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 14, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(3.0),
                Value::str("99"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['data:text/plain,a', 3, '99']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 14, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(4.0),
                Value::str("name"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['data:text/plain,a', 4, 'name']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 14, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(5.0),
                Value::str("pw"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['data:text/plain,a', 5, 'pw']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 14, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(6.0),
                Value::str("/path"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['data:text/plain,a', 6, '/path']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 14, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(7.0),
                Value::str("?x"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("data:text/plain,a?x")),
            "reference update ['data:text/plain,a', 7, '?x']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 17, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("data:text/plain,a"),
                Value::Number(8.0),
                Value::str("#y"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("data:text/plain,a#y")),
            "reference update ['data:text/plain,a', 8, '#y']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 17, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(0.0),
                Value::str("http:"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['custom:opaque', 0, 'http:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 17, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(1.0),
                Value::str("host:2"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['custom:opaque', 1, 'host:2']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 17, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(2.0),
                Value::str("host"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['custom:opaque', 2, 'host']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 17, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(3.0),
                Value::str("99"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['custom:opaque', 3, '99']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 17, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(4.0),
                Value::str("name"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['custom:opaque', 4, 'name']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 17, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(5.0),
                Value::str("pw"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['custom:opaque', 5, 'pw']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 17, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(6.0),
                Value::str("/path"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['custom:opaque', 6, '/path']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 5, 5, 5, 4294967295, 5, 4294967295, 17, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(7.0),
                Value::str("?x"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom:opaque?x")),
            "reference update ['custom:opaque', 7, '?x']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 7, 7, 7, 4294967295, 7, 13, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom:opaque"),
                Value::Number(8.0),
                Value::str("#y"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom:opaque#y")),
            "reference update ['custom:opaque', 8, '#y']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 7, 7, 7, 4294967295, 7, 4294967295, 13, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(0.0),
                Value::str("http:"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::Boolean(false)),
            "reference update ['custom://host/path', 0, 'http:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 7, 7, 7, 4294967295, 7, 4294967295, 13, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(1.0),
                Value::str("host:2"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom://host:2/path")),
            "reference update ['custom://host/path', 1, 'host:2']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 2, 15, 4294967295, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(2.0),
                Value::str("host"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom://host/path")),
            "reference update ['custom://host/path', 2, 'host']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 4294967295, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(3.0),
                Value::str("99"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom://host:99/path")),
            "reference update ['custom://host/path', 3, '99']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 99, 16, 4294967295, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(4.0),
                Value::str("name"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom://name@host/path")),
            "reference update ['custom://host/path', 4, 'name']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 13, 13, 18, 4294967295, 18, 4294967295, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(5.0),
                Value::str("pw"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom://:pw@host/path")),
            "reference update ['custom://host/path', 5, 'pw']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 12, 17, 4294967295, 17, 4294967295, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(6.0),
                Value::str("/path"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom://host/path")),
            "reference update ['custom://host/path', 6, '/path']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 4294967295, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(7.0),
                Value::str("?x"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom://host/path?x")),
            "reference update ['custom://host/path', 7, '?x']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 18, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/path"),
                Value::Number(8.0),
                Value::str("#y"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("custom://host/path#y")),
            "reference update ['custom://host/path', 8, '#y']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 4294967295, 18, 1]
        );
    }
    assert!(
        ops::strict_equals(
            &call(&mut realm, &binding, "domainToASCII", &[Value::str("")]).unwrap(),
            &Value::str("")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(&mut realm, &binding, "domainToUnicode", &[Value::str("")]).unwrap(),
            &Value::str("")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToASCII",
                &[Value::str("Exämple.com")]
            )
            .unwrap(),
            &Value::str("xn--exmple-cua.com")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("Exämple.com")]
            )
            .unwrap(),
            &Value::str("exämple.com")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToASCII",
                &[Value::str("faß.de")]
            )
            .unwrap(),
            &Value::str("xn--fa-hia.de")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("faß.de")]
            )
            .unwrap(),
            &Value::str("faß.de")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToASCII",
                &[Value::str("xn--fa-hia.de")]
            )
            .unwrap(),
            &Value::str("xn--fa-hia.de")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("xn--fa-hia.de")]
            )
            .unwrap(),
            &Value::str("faß.de")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToASCII",
                &[Value::str("0x7f000001")]
            )
            .unwrap(),
            &Value::str("127.0.0.1")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("0x7f000001")]
            )
            .unwrap(),
            &Value::str("127.0.0.1")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToASCII",
                &[Value::str("[::1]")]
            )
            .unwrap(),
            &Value::str("[::1]")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("[::1]")]
            )
            .unwrap(),
            &Value::str("[::1]")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(&mut realm, &binding, "domainToASCII", &[Value::str("a:b")]).unwrap(),
            &Value::str("")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("a:b")]
            )
            .unwrap(),
            &Value::str("")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToASCII",
                &[Value::str("bad host")]
            )
            .unwrap(),
            &Value::str("")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("bad host")]
            )
            .unwrap(),
            &Value::str("")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToASCII",
                &[Value::str("example.com/path")]
            )
            .unwrap(),
            &Value::str("example.com")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("example.com/path")]
            )
            .unwrap(),
            &Value::str("example.com")
        ),
        "domainToUnicode case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToASCII",
                &[Value::str("%65xample.com")]
            )
            .unwrap(),
            &Value::str("example.com")
        ),
        "domainToASCII case"
    );
    assert!(
        ops::strict_equals(
            &call(
                &mut realm,
                &binding,
                "domainToUnicode",
                &[Value::str("%65xample.com")]
            )
            .unwrap(),
            &Value::str("example.com")
        ),
        "domainToUnicode case"
    );
    {
        let result = call(
            &mut realm,
            &binding,
            "getOrigin",
            &[Value::str("http://u:p@host:80/a")],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("http://host")),
            "reference getOrigin ['http://u:p@host:80/a']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 4294967295, 18, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "getOrigin",
            &[Value::str("https://host/a")],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("https://host")),
            "reference getOrigin ['https://host/a']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 4294967295, 18, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "getOrigin",
            &[Value::str("file:///C:/a")],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("null")),
            "reference getOrigin ['file:///C:/a']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 4294967295, 18, 1]
        );
    }
    {
        let result = call(&mut realm, &binding, "getOrigin", &[Value::str("data:a")]);
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("null")),
            "reference getOrigin ['data:a']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 4294967295, 18, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "getOrigin",
            &[Value::str("blob:https://example.com/id")],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("https://example.com")),
            "reference getOrigin ['blob:https://example.com/id']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 13, 4294967295, 13, 4294967295, 18, 1]
        );
    }
    {
        let result = call(&mut realm, &binding, "getOrigin", &[Value::str("bad")]);
        let error = result.unwrap_err().0;
        assert!(ops::strict_equals(
            &value(&mut realm, &error, "code"),
            &Value::str("ERR_INVALID_URL")
        ));
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "pathToFileURL",
            &[
                Value::str("/a b/#x?y%z"),
                Value::Boolean(false),
                Value::Undefined,
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("file:///a%20b/%23x%3Fy%25z")),
            "reference pathToFileURL ['/a b/#x?y%z', False, None]"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "pathToFileURL",
            &[
                Value::str("C:a b#x?y%z"),
                Value::Boolean(true),
                Value::Undefined,
            ],
        );
        let error = result.unwrap_err().0;
        assert!(ops::strict_equals(
            &value(&mut realm, &error, "code"),
            &Value::str("ERR_INVALID_URL")
        ));
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "pathToFileURL",
            &[
                Value::str("/share/a b"),
                Value::Boolean(true),
                Value::str("server"),
            ],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("file://server/share/a%20b")),
            "reference pathToFileURL ['/share/a b', True, 'server']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 13, 4294967295, 13, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "pathToFileURL",
            &[Value::str("/你好"), Value::Boolean(false), Value::Undefined],
        );
        assert!(
            ops::strict_equals(&result.unwrap(), &Value::str("file:///%E4%BD%A0%E5%A5%BD")),
            "reference pathToFileURL ['/\\u4f60\\u597d', False, None]"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
        );
    }
}

#[test]
fn binding_format_options_and_internal_contract_failures() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    let input = "https://user:pw@xn--exmple-cua.com:42/a?q#f";
    for (flags, expected) in [
        (
            [true, true, true, true],
            "https://user:pw@exämple.com:42/a?q#f",
        ),
        (
            [false, false, false, false],
            "https://xn--exmple-cua.com:42/a",
        ),
        ([true, false, true, true], input),
    ] {
        let mut args = vec![Value::str(input)];
        args.extend(flags.map(Value::Boolean));
        let result = call(&mut realm, &binding, "format", &args).unwrap();
        assert!(ops::strict_equals(&result, &Value::str(expected)));
    }
    let result = call(
        &mut realm,
        &binding,
        "format",
        &[
            Value::str("data:a?q#h"),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(false),
        ],
    )
    .unwrap();
    assert!(ops::strict_equals(&result, &Value::str("data:a?q#h")));
    for name in [
        "update",
        "domainToASCII",
        "domainToUnicode",
        "format",
        "pathToFileURL",
    ] {
        assert!(call(&mut realm, &binding, name, &[]).is_err());
    }
    for name in ["format", "update"] {
        assert!(
            call(
                &mut realm,
                &binding,
                name,
                &[Value::str("bad"), Value::Number(0.0), Value::str("x")]
            )
            .is_err()
        );
    }
    let href = Value::str("https://host/a");
    assert!(
        call(
            &mut realm,
            &binding,
            "update",
            &[href.clone(), Value::Undefined]
        )
        .is_err()
    );
    let symbol = Value::Symbol(crate::Symbol::new(Some(crate::JsStr::from("action"))));
    assert!(
        call(
            &mut realm,
            &binding,
            "update",
            &[href.clone(), symbol, Value::str("x")]
        )
        .is_err()
    );
    let result = call(
        &mut realm,
        &binding,
        "update",
        &[href, Value::Number(99.0), Value::str("x")],
    )
    .unwrap();
    assert!(ops::strict_equals(&result, &Value::Boolean(false)));
    for args in [
        vec![Value::str(""), Value::Boolean(false)],
        vec![Value::str("host:42"), Value::Boolean(false)],
        vec![
            Value::str("/a"),
            Value::Boolean(true),
            Value::str("bad host"),
        ],
    ] {
        assert!(call(&mut realm, &binding, "pathToFileURL", &args).is_err());
    }
    let mut url = Url::parse("http://host:42/a").unwrap();
    assert!(!host(&mut url, "[bad", false));
    assert!(!host(&mut url, "bad host", false));
    assert!(host(&mut url, "h\tost:/rest", false));
    assert_eq!(url.as_str(), "http://host:42/a");
    assert!(!update(&mut url, 0, "!invalid:"));
    assert!(update(&mut url, 7, "q"));
    assert!(update(&mut url, 8, "h"));
    assert_eq!(url.as_str(), "http://host:42/a?q#h");
}

#[test]
fn node_22_protocol_and_host_edge_cases() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    let buffer = value(&mut realm, &Value::Object(binding.clone()), "urlComponents");
    let view = typed_array_builtin::view_of(&buffer).unwrap();
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host/a"),
                Value::Number(0.0),
                Value::str("file:"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::str("file://host/a")),
            "['https://host/a', 0, 'file:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host:44/a"),
                Value::Number(0.0),
                Value::str("file:"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['https://host:44/a', 0, 'file:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 6]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file://host/a"),
                Value::Number(0.0),
                Value::str("http:"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::str("http://host/a")),
            "['file://host/a', 0, 'http:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 0]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///a"),
                Value::Number(0.0),
                Value::str("file:"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['file:///a', 0, 'file:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 0]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("http://host/a"),
                Value::Number(0.0),
                Value::str("1bad:"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['http://host/a', 0, '1bad:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 0]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("http://host/a"),
                Value::Number(0.0),
                Value::str("h\tttp:"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::str("http://host/a")),
            "['http://host/a', 0, 'h\\tttp:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 0]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host/a"),
                Value::Number(1.0),
                Value::str("[::1]junk"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['https://host/a', 1, '[::1]junk']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 0]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host/a"),
                Value::Number(2.0),
                Value::str("[::1]junk"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['https://host/a', 2, '[::1]junk']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 11, 4294967295, 11, 4294967295, 4294967295, 0]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host/a"),
                Value::Number(1.0),
                Value::str("[::1]:"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::str("https://[::1]/a")),
            "['https://host/a', 1, '[::1]:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 13, 4294967295, 13, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host:42/a"),
                Value::Number(1.0),
                Value::str("other:"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::str("https://other:42/a")),
            "['https://host:42/a', 1, 'other:']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 13, 42, 16, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host/a"),
                Value::Number(3.0),
                Value::str("junk"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['https://host/a', 3, 'junk']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 13, 42, 16, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host/a"),
                Value::Number(3.0),
                Value::str(" 99"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['https://host/a', 3, ' 99']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 13, 42, 16, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host:42/a"),
                Value::Number(1.0),
                Value::str(""),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['custom://host:42/a', 1, '']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![6, 8, 8, 13, 42, 16, 4294967295, 4294967295, 2]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("custom://host/a"),
                Value::Number(1.0),
                Value::str(""),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::str("custom:///a")),
            "['custom://host/a', 1, '']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 9, 4294967295, 9, 4294967295, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("https://host/a"),
                Value::Number(1.0),
                Value::str(""),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::Boolean(false)),
            "['https://host/a', 1, '']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![7, 9, 9, 9, 4294967295, 9, 4294967295, 4294967295, 1]
        );
    }
    {
        let result = call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file:///a"),
                Value::Number(1.0),
                Value::str("localhost"),
            ],
        )
        .unwrap();
        assert!(
            ops::strict_equals(&result, &Value::str("file:///a")),
            "['file:///a', 1, 'localhost']"
        );
        assert_eq!(
            (0..9)
                .map(|i| component(&view, i as f64))
                .collect::<Vec<_>>(),
            vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
        );
    }
}

#[test]
fn node_22_format_flags_and_encoded_path_characters() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    let buffer = value(&mut realm, &Value::Object(binding.clone()), "urlComponents");
    let view = typed_array_builtin::view_of(&buffer).unwrap();
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        &Value::str("https://xn--exmple-cua.com:42/a")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        &Value::str("https://xn--exmple-cua.com:42/a#f")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        &Value::str("https://exämple.com:42/a")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        &Value::str("https://exämple.com:42/a#f")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        &Value::str("https://xn--exmple-cua.com:42/a?q")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        &Value::str("https://xn--exmple-cua.com:42/a?q#f")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        &Value::str("https://exämple.com:42/a?q")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        &Value::str("https://exämple.com:42/a?q#f")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        &Value::str("https://user:pw@xn--exmple-cua.com:42/a")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        &Value::str("https://user:pw@xn--exmple-cua.com:42/a#f")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        &Value::str("https://user:pw@exämple.com:42/a")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        &Value::str("https://user:pw@exämple.com:42/a#f")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        &Value::str("https://user:pw@xn--exmple-cua.com:42/a?q")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        &Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        &Value::str("https://user:pw@exämple.com:42/a?q")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "format",
            &[
                Value::str("https://user:pw@xn--exmple-cua.com:42/a?q#f"),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        &Value::str("https://user:pw@exämple.com:42/a?q#f")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "pathToFileURL",
            &[
                Value::str("C:\\a b\\#x?y%z"),
                Value::Boolean(true),
                Value::Undefined
            ]
        )
        .unwrap(),
        &Value::str("file:///C:/a%20b/%23x%3Fy%25z")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "pathToFileURL",
            &[
                Value::str("C:\\a\\[]^|~%\\你好"),
                Value::Boolean(true),
                Value::str("server")
            ]
        )
        .unwrap(),
        &Value::str("file://server/C:/a/%5B%5D%5E%7C%7E%25/%E4%BD%A0%E5%A5%BD")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 13, 4294967295, 13, 4294967295, 4294967295, 6]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "pathToFileURL",
            &[Value::str("/a\\b"), Value::Boolean(false), Value::Undefined]
        )
        .unwrap(),
        &Value::str("file:///a%5Cb")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "pathToFileURL",
            &[
                Value::str("/a\0\t\n\r"),
                Value::Boolean(false),
                Value::Undefined
            ]
        )
        .unwrap(),
        &Value::str("file:///a%00%09%0A%0D")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
    );
}

#[test]
fn node_22_network_file_drive_hosts_survive_parse_and_base_resolution() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    let buffer = value(&mut realm, &Value::Object(binding.clone()), "urlComponents");
    let view = typed_array_builtin::view_of(&buffer).unwrap();
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "parse",
            &[Value::str("file://server/C:/a"), Value::Undefined]
        )
        .unwrap(),
        &Value::str("file://server/C:/a")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 13, 4294967295, 13, 4294967295, 4294967295, 6]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "parse",
            &[Value::str("/C:/b"), Value::str("file://server/share/a")]
        )
        .unwrap(),
        &Value::str("file://server/C:/b")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 13, 4294967295, 13, 4294967295, 4294967295, 6]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "parse",
            &[
                Value::str("file:/C:/b"),
                Value::str("file://server/share/a")
            ]
        )
        .unwrap(),
        &Value::str("file://server/C:/b")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 13, 4294967295, 13, 4294967295, 4294967295, 6]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "parse",
            &[
                Value::str("file:///C:/b"),
                Value::str("file://server/share/a")
            ]
        )
        .unwrap(),
        &Value::str("file:///C:/b")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "parse",
            &[Value::str("C:/b"), Value::str("file://server/share/a")]
        )
        .unwrap(),
        &Value::str("c:/b")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![2, 2, 2, 2, 4294967295, 2, 4294967295, 4294967295, 1]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "parse",
            &[Value::str("../C:/b"), Value::str("file://server/share/a")]
        )
        .unwrap(),
        &Value::str("file://server/C:/b")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 13, 4294967295, 13, 4294967295, 4294967295, 6]
    );
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "update",
            &[
                Value::str("file://server/C:/a"),
                Value::Number(2.0),
                Value::str("")
            ]
        )
        .unwrap(),
        &Value::str("file:///C:/a")
    ));
    assert_eq!(
        (0..9)
            .map(|i| component(&view, i as f64))
            .collect::<Vec<_>>(),
        vec![5, 7, 7, 7, 4294967295, 7, 4294967295, 4294967295, 6]
    );
}

#[test]
fn file_hosts_do_not_inherit_from_other_schemes_and_survive_protocol_changes() {
    assert_eq!(
        parse("file:/C:/a", Some("https://host/a"))
            .unwrap()
            .as_str(),
        "file:///C:/a"
    );
    let mut url = parse("https://server/C:/a", None).unwrap();
    assert!(update(&mut url, 0, "file:"));
    assert_eq!(url.as_str(), "file://server/C:/a");
}

#[test]
fn private_encoding_idna_matches_node_22_including_ascii_alabels() {
    let mut realm = Realm::new();
    let binding = encoding(&mut realm);
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("")]).unwrap(),
        &Value::str("")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "toASCII",
            &[Value::str("EXAMPLE.COM")]
        )
        .unwrap(),
        &Value::str("example.com")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "toASCII",
            &[Value::str("Exämple.com")]
        )
        .unwrap(),
        &Value::str("xn--exmple-cua.com")
    ));
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("faß.de")]).unwrap(),
        &Value::str("xn--fa-hia.de")
    ));
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("bad host")]).unwrap(),
        &Value::str("bad host")
    ));
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("a:b")]).unwrap(),
        &Value::str("a:b")
    ));
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("xn--bad-")]).unwrap(),
        &Value::str("xn--bad-")
    ));
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("xn--")]).unwrap(),
        &Value::str("")
    ));
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("xn--a?")]).unwrap(),
        &Value::str("")
    ));
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("xn--dca")]).unwrap(),
        &Value::str("")
    ));
    assert!(ops::strict_equals(
        &call(&mut realm, &binding, "toASCII", &[Value::str("xn--9ca")]).unwrap(),
        &Value::str("xn--9ca")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "toASCII",
            &[Value::str("xn--bcher-kva")]
        )
        .unwrap(),
        &Value::str("xn--bcher-kva")
    ));
    assert!(ops::strict_equals(
        &call(
            &mut realm,
            &binding,
            "toASCII",
            &[Value::String(crate::JsStr::from_units(&[0xd800]))]
        )
        .unwrap(),
        &Value::str("")
    ));
    assert!(call(&mut realm, &binding, "toASCII", &[]).is_err());
}
