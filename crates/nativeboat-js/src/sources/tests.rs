use super::*;

#[test]
fn source_functions_validate_indexes_and_preserve_global_evaluation() {
    let context = &mut Context::default();
    let sources = Sources {
        code: Rc::new(vec![
            "const globalNativeSource = 7; globalNativeSource".into(),
            "const =".into(),
        ]),
    };
    let args = |index: i32| {
        vec![
            index.into(),
            JsString::from("native-source.js").into(),
            JsString::from("").into(),
        ]
    };
    let value = evaluate(&JsValue::undefined(), &args(0), &sources, context).unwrap();
    assert_eq!(value.as_number(), Some(7.0));
    assert_eq!(
        context
            .eval(Source::from_bytes("globalNativeSource"))
            .unwrap()
            .as_number(),
        Some(7.0)
    );
    assert!(evaluate(&JsValue::undefined(), &args(1), &sources, context).is_err());
    assert!(evaluate(&JsValue::undefined(), &args(2), &sources, context).is_err());
    assert!(compile(&JsValue::undefined(), &args(-1), &sources, context).is_err());
    let factory = compile(&JsValue::undefined(), &args(0), &sources, context).unwrap();
    assert!(factory.as_callable().is_some());
}
