use super::*;

#[test]
fn journal_handles_reject_other_values_and_missing_nodes() {
    let dom = RefCell::new(Dom::new());
    let mut context = Context::default();
    let invalid = JsValue::from(JsObject::with_null_proto());
    assert!(journal(&invalid).is_err());
    assert!(journal(&JsValue::null()).is_err());
    assert!(begin(&dom, &[JsValue::from(u32::MAX)], &mut context).is_err());
    assert!(changes(&dom, &[JsValue::from(u32::MAX)], &mut context).is_err());
    assert!(revert(&dom, core::slice::from_ref(&invalid), &mut context).is_err());
    let id = begin(&dom, &[JsValue::from(Dom::DOCUMENT.0)], &mut context).unwrap();
    assert!(
        end(&dom, core::slice::from_ref(&id), &mut context)
            .unwrap()
            .is_null()
    );
    assert!(end(&dom, &[id], &mut context).unwrap().is_null());
    let empty = wrap(Journal::default());
    assert!(join(&dom, &[empty.clone(), invalid.clone()], &mut context).is_err());
    assert!(join(&dom, &[invalid, empty.clone()], &mut context).is_err());
    assert!(join(&dom, &[empty.clone(), empty], &mut context).is_ok());
}
