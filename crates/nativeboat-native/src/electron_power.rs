//! Electron's `powerSaveBlocker`: keeps the computer, or also the display,
//! from sleeping while any blocker is started, the strongest one winning.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use crate::{ErrorKind, Key, Object, Property, Realm, Value, ops};

const ES_CONTINUOUS: u32 = 0x8000_0000;
const ES_SYSTEM_REQUIRED: u32 = 0x1;
const ES_DISPLAY_REQUIRED: u32 = 0x2;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetThreadExecutionState(flags: u32) -> u32;
}

/// The execution state for the started blockers: whether any keeps the
/// display on, and whether any is started at all.
fn execution_state(blockers: &[(u32, bool)]) -> u32 {
    if blockers.iter().any(|(_, display)| *display) {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
    } else if blockers.is_empty() {
        ES_CONTINUOUS
    } else {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    }
}

fn apply(blockers: &[(u32, bool)]) {
    // SAFETY: only sets the calling thread's execution state.
    unsafe { SetThreadExecutionState(execution_state(blockers)) };
}

/// Electron's `powerSaveBlocker` module. Blockers are numbered from 0, as
/// Electron numbers them, and a number is not given out again.
pub(crate) fn module(realm: &Realm) -> Object {
    let blocker = realm.object();
    let started: Rc<RefCell<Vec<(u32, bool)>>> = Rc::default();
    let next = Rc::new(Cell::new(0u32));
    let list = started.clone();
    let start = realm.function("start", 1, false, move |realm, _, args| {
        let kind = match args.first() {
            Some(Value::String(kind)) => kind.to_rust_lossy(),
            _ => alloc::string::String::new(),
        };
        let display = match kind.as_str() {
            "prevent-app-suspension" => false,
            "prevent-display-sleep" => true,
            _ => {
                return Err(realm.throw(
                    ErrorKind::Type,
                    "Error processing argument at index 0, conversion failure from the power save blocker type",
                ));
            }
        };
        let id = next.get();
        next.set(id + 1);
        list.borrow_mut().push((id, display));
        apply(&list.borrow());
        Ok(Value::Number(f64::from(id)))
    });
    let list = started.clone();
    let stop = realm.function("stop", 1, false, move |realm, _, args| {
        let id = ops::to_number(realm, args.first().unwrap_or(&Value::Undefined))?;
        let before = list.borrow().len();
        list.borrow_mut()
            .retain(|(started, _)| f64::from(*started) != id);
        let stopped = list.borrow().len() != before;
        apply(&list.borrow());
        Ok(Value::Boolean(stopped))
    });
    let list = started;
    let is_started = realm.function("isStarted", 1, false, move |realm, _, args| {
        let id = ops::to_number(realm, args.first().unwrap_or(&Value::Undefined))?;
        Ok(Value::Boolean(
            list.borrow()
                .iter()
                .any(|(started, _)| f64::from(*started) == id),
        ))
    });
    for (name, function) in [("start", start), ("stop", stop), ("isStarted", is_started)] {
        blocker.define(Key::name(name), Property::data(Value::Object(function)));
    }
    blocker
}

#[cfg(test)]
#[path = "electron_power/tests.rs"]
mod tests;
