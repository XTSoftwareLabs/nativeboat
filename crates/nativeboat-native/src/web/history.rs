//! Native edit history: journals of the changes edits make to an editing
//! host, which undo and redo revert in Rust.

use super::node_at;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Completion, ErrorKind, Kind, Object, Realm, Value};
use alloc::rc::Rc;
use core::cell::RefCell;
use nativeboat_dom::{Dom, Journal};

fn journal(realm: &Realm, value: &Value) -> Result<Rc<dyn core::any::Any>, crate::Thrown> {
    if let Value::Object(object) = value
        && let Kind::Native(data) = &object.data().kind
        && data.is::<Journal>()
    {
        return Ok(data.clone());
    }
    Err(realm.throw(ErrorKind::Type, "not an edit journal"))
}

fn contents(data: &Rc<dyn core::any::Any>) -> &Journal {
    data.downcast_ref::<Journal>()
        .expect("a checked edit journal")
}

fn wrap(journal: Journal) -> Value {
    let data: Rc<dyn core::any::Any> = Rc::new(journal);
    Value::Object(Object::new(None, Kind::Native(data)))
}

/// Starts recording an editing host's changes; returns the recording.
pub(super) fn begin(dom: &RefCell<Dom>, realm: &mut Realm, args: &[Value]) -> Completion {
    let host = node_at(realm, &dom.borrow(), args, 0)?;
    Ok(Value::Number(dom.borrow_mut().begin_journal(host) as f64))
}

/// Ends a recording: the journal of what changed, or null when nothing did.
pub(super) fn end(dom: &RefCell<Dom>, realm: &mut Realm, args: &[Value]) -> Completion {
    let id = super::number(realm, args, 0)? as u64;
    Ok(match dom.borrow_mut().end_journal(id) {
        Some(journal) if !journal.is_empty() => wrap(journal),
        _ => Value::Null,
    })
}

/// How often an editing host's content has changed.
pub(super) fn changes(dom: &RefCell<Dom>, realm: &mut Realm, args: &[Value]) -> Completion {
    let host = node_at(realm, &dom.borrow(), args, 0)?;
    Ok(Value::Number(dom.borrow().host_changes(host) as f64))
}

/// Undoes a journal's changes.
pub(super) fn revert(dom: &RefCell<Dom>, realm: &mut Realm, args: &[Value]) -> Completion {
    let data = journal(realm, super::arg(args, 0))?;
    dom.borrow_mut().revert_journal(contents(&data));
    Ok(Value::Undefined)
}

/// One journal followed by another, as one edit.
pub(super) fn join(_: &RefCell<Dom>, realm: &mut Realm, args: &[Value]) -> Completion {
    let first = journal(realm, super::arg(args, 0))?;
    let second = journal(realm, super::arg(args, 1))?;
    Ok(wrap(contents(&first).then(contents(&second))))
}
