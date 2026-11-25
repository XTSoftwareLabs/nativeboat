//! Native edit history: journals of the changes edits make to an editing
//! host, which undo and redo revert in Rust.
use super::*;
use boa_engine::{Finalize, JsData, JsObject, Trace};
use nativeboat_dom::Journal;

#[derive(Debug, Trace, Finalize, JsData)]
struct Recorded {
    #[unsafe_ignore_trace]
    journal: Journal,
}

fn journal(value: &JsValue) -> JsResult<JsObject<Recorded>> {
    value
        .as_object()
        .and_then(|object| object.clone().downcast::<Recorded>().ok())
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("not an edit journal")
                .into()
        })
}

fn wrap(journal: Journal) -> JsValue {
    JsObject::from_proto_and_data(None, Recorded { journal }).into()
}

pub(super) fn begin(dom: &RefCell<Dom>, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let host = node_at(&dom.borrow(), args, 0)?;
    Ok(JsValue::from(dom.borrow_mut().begin_journal(host) as f64))
}

pub(super) fn end(
    dom: &RefCell<Dom>,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id = number(args, 0, context)? as u64;
    Ok(match dom.borrow_mut().end_journal(id) {
        Some(journal) if !journal.is_empty() => wrap(journal),
        _ => JsValue::null(),
    })
}

pub(super) fn changes(dom: &RefCell<Dom>, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let host = node_at(&dom.borrow(), args, 0)?;
    Ok(JsValue::from(dom.borrow().host_changes(host) as f64))
}

pub(super) fn revert(dom: &RefCell<Dom>, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let recorded = journal(args.get_or_undefined(0))?;
    let journal = recorded.borrow().data().journal.clone();
    dom.borrow_mut().revert_journal(&journal);
    Ok(JsValue::undefined())
}

pub(super) fn join(_: &RefCell<Dom>, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let first = journal(args.get_or_undefined(0))?;
    let second = journal(args.get_or_undefined(1))?;
    let joined = first
        .borrow()
        .data()
        .journal
        .then(&second.borrow().data().journal);
    Ok(wrap(joined))
}

#[cfg(test)]
mod tests;
