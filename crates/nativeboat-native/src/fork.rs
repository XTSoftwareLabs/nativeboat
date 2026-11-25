//! Modules a program starts as child processes with `fork()`, as Electron's
//! `utilityProcess.fork` and Node's `child_process.fork` start them. The
//! compiler links each forked module as a root of the program's main graph;
//! the program registers the paths each is started by, and a child process
//! started for one runs that root instead of the main module, with
//! `process.parentPort` connected to its parent.
//!
//! The parent side and the messages are the Node library's
//! `internal/electron/utility_process`, over the `child_process` and `net`
//! modules: the child is the program's own executable, started with the
//! fork's index and a loopback port and token in its environment.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::{Key, Object, Realm, Thrown, Value, ops};
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// The environment variable naming the fork a child process runs.
pub const ENTRY_VARIABLE: &str = "NATIVEBOAT_FORK_ENTRY";

static TABLE: AtomicPtr<&'static [&'static str]> = AtomicPtr::new(core::ptr::null_mut());
static TABLE_LENGTH: AtomicUsize = AtomicUsize::new(0);

/// Registers the paths each forked module is started by, lowercase with
/// `/` separators, in the order of the main graph's roots after the main
/// module.
pub fn register(forks: &'static [&'static [&'static str]]) {
    TABLE_LENGTH.store(forks.len(), Ordering::Release);
    TABLE.store(forks.as_ptr().cast_mut(), Ordering::Release);
}

/// The registered forks.
fn table() -> &'static [&'static [&'static str]] {
    let pointer = TABLE.load(Ordering::Acquire);
    if pointer.is_null() {
        return &[];
    }
    // SAFETY: `register` stored a pointer and length of a static slice.
    unsafe { core::slice::from_raw_parts(pointer, TABLE_LENGTH.load(Ordering::Acquire)) }
}

/// The fork a module path names: the one whose start path ends it, as a
/// window's URL names a page. Paths compare with `/` separators,
/// case-insensitively, at a path boundary.
pub fn entry_for(path: &str) -> Option<usize> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    table().iter().position(|paths| {
        paths.iter().any(|known| {
            path.strip_suffix(known)
                .is_some_and(|before| before.is_empty() || before.ends_with('/'))
                || known
                    .strip_suffix(&*path)
                    .is_some_and(|before| before.ends_with('/'))
        })
    })
}

/// The fork this process was started to run, from its environment.
pub fn requested_entry() -> Option<usize> {
    crate::platform::environment_value(&crate::JsStr::from(ENTRY_VARIABLE))?
        .to_rust_lossy()
        .trim()
        .parse()
        .ok()
}

/// Runs a forked child process: connects it to its parent, then evaluates
/// the main graph's root `entry` and runs the event loop, as the main
/// process runs its main module.
pub fn run(
    realm: &mut Realm,
    entry: usize,
    initialize: fn(&mut Realm, usize) -> Result<Vec<Object>, Thrown>,
) -> i32 {
    realm.run_main(move |realm| {
        connect_parent(realm)?;
        initialize(realm, entry)
    })
}

/// The connection to the parent, from the Node library's utility process
/// module: `process.parentPort` for a utility process, Node's IPC channel
/// (`process.send` and `message` events) for a `child_process.fork` child.
fn connect_parent(realm: &mut Realm) -> Result<(), Thrown> {
    let module = crate::modules::load(realm, "node:internal/electron/utility_process")?;
    let connect = ops::get_named(realm, &module, "connectParent")?;
    ops::call(realm, &connect, module, &[])?;
    Ok(())
}

/// The fork table as the utility process module reads it: the start paths
/// of each fork, in order.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let binding = realm.object();
    let entry = realm.function("entryFor", 1, false, |realm, _, args| {
        let path =
            ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?.to_rust_lossy();
        Ok(entry_for(&path).map_or(Value::Number(-1.0), |entry| Value::Number(entry as f64)))
    });
    binding.define(
        Key::name("entryFor"),
        crate::Property::data(Value::Object(entry)),
    );
    let variable = realm.function("entryVariable", 0, false, |_, _, _| {
        Ok(Value::str(ENTRY_VARIABLE))
    });
    binding.define(
        Key::name("entryVariable"),
        crate::Property::data(Value::Object(variable)),
    );
    binding
}
