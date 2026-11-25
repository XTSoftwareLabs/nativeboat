//! Polling for Node's unchanged `fs.watchFile`/chokidar polling paths.
//!
//! Follows Node 22 StatWatcher and libuv fs-poll: the first successful stat
//! establishes a baseline; errors and subsequent changes emit current/previous
//! statistics. Only one stat is outstanding, and waiting uses the event loop
//! rather than occupying a worker thread for the polling interval.

use super::*;
use core::cell::{Cell, RefCell};

struct Watcher {
    active: bool,
    closed: bool,
    path: Vec<u16>,
    interval: u32,
    status: i32,
    previous: crate::fs_win::Stat,
    timer: Option<Object>,
    referenced: Rc<Cell<bool>>,
}

fn changed(previous: &crate::fs_win::Stat, current: &crate::fs_win::Stat) -> bool {
    // Access time, nlink and block allocation alone do not trigger uv_fs_poll.
    previous.mode != current.mode
        || previous.ino != current.ino
        || previous.dev != current.dev
        || previous.size != current.size
        || previous.times[1..] != current.times[1..]
}

fn poll(
    realm: &mut Realm,
    object: Object,
    watcher: Rc<RefCell<Watcher>>,
    binding: Rc<Binding>,
    bigint: bool,
) -> Result<(), u32> {
    let started = (realm.event_loop.clock)();
    let (path, referenced) = {
        let watcher = watcher.borrow();
        (watcher.path.clone(), watcher.referenced.clone())
    };
    realm.submit_io_with_ref(
        move || crate::fs_win::stat(&path, false),
        referenced,
        move |realm, result| {
            if !watcher.borrow().active {
                return Ok(());
            }
            let event = {
                let mut state = watcher.borrow_mut();
                match result {
                    Err(status) => {
                        let emit = (state.status != status).then(|| {
                            (
                                status,
                                crate::fs_win::Stat::default(),
                                state.previous.clone(),
                            )
                        });
                        state.status = status;
                        emit
                    }
                    Ok(current) => {
                        let emit = (state.status != 0
                            && (state.status < 0 || changed(&state.previous, &current)))
                        .then(|| (0, current.clone(), state.previous.clone()));
                        state.status = 1;
                        state.previous = current;
                        emit
                    }
                }
            };
            let result = if let Some((status, current, previous)) = event {
                let mut words = [0; 36];
                words[..18].copy_from_slice(&stat_words(current));
                words[18..].copy_from_slice(&stat_words(previous));
                let stats = binding.stats.publish(realm, &words, bigint, false);
                ops::get(
                    realm,
                    &Value::Object(object.clone()),
                    &Key::name("onchange"),
                )
                .and_then(|callback| {
                    ops::call(
                        realm,
                        &callback,
                        Value::Object(object.clone()),
                        &[Value::Number(f64::from(status)), stats],
                    )
                })
                .map(drop)
            } else {
                Ok(())
            };
            // Even a handled source exception must not silently stop polling.
            if watcher.borrow().active {
                let (interval, referenced) = {
                    let state = watcher.borrow();
                    (f64::from(state.interval), state.referenced.clone())
                };
                let elapsed = ((realm.event_loop.clock)() - started).max(0.0);
                let delay = interval - elapsed % interval;
                let next = watcher.clone();
                let callback = realm.function("poll", 0, false, move |realm, _, _| {
                    next.borrow_mut().timer = None;
                    if let Err(error) =
                        poll(realm, object.clone(), next.clone(), binding.clone(), bigint)
                    {
                        next.borrow_mut().active = false;
                        next.borrow().referenced.set(false);
                        return Err(realm.throw(
                            ErrorKind::Error,
                            &format!("Could not submit filesystem poll: {error}"),
                        ));
                    }
                    Ok(Value::Undefined)
                });
                let timer = realm.native_timeout(callback, delay, referenced);
                watcher.borrow_mut().timer = Some(timer);
            }
            result
        },
    )
}

pub(super) fn constructor(realm: &mut Realm, binding: Rc<Binding>) -> Object {
    realm.function("StatWatcher", 1, true, move |realm, this, args| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(ErrorKind::Type, "StatWatcher requires construction"));
        }
        let object = ops::to_object(realm, &this)?;
        let bigint = matches!(args.first(), Some(Value::Boolean(true)));
        let watcher = Rc::new(RefCell::new(Watcher {
            active: false,
            closed: false,
            path: Vec::new(),
            interval: 1,
            status: 0,
            previous: crate::fs_win::Stat::default(),
            timer: None,
            referenced: Rc::new(Cell::new(true)),
        }));
        let state = watcher.clone();
        let start_binding = binding.clone();
        realm.method(&object, "start", 2, move |realm, this, args| {
            let mut watcher = state.borrow_mut();
            if watcher.active || watcher.closed {
                return Ok(Value::Undefined);
            }
            let Value::String(path) = arg(args, 0) else {
                return Ok(Value::Number(f64::from(crate::fs_win::UV_EINVAL)));
            };
            let Value::Number(interval) = arg(args, 1) else {
                return Ok(Value::Number(f64::from(crate::fs_win::UV_EINVAL)));
            };
            if path.units().contains(&0) || interval != f64::from(interval as u32) {
                return Ok(Value::Number(f64::from(crate::fs_win::UV_EINVAL)));
            }
            let object = ops::to_object(realm, &this)?;
            watcher.path = path.units().to_vec();
            watcher.path.push(0);
            watcher.interval = (interval as u32).max(1);
            watcher.active = true;
            drop(watcher);
            if let Err(error) = poll(realm, object, state.clone(), start_binding.clone(), bigint) {
                state.borrow_mut().active = false;
                return Ok(Value::Number(f64::from(crate::uv_error::translate(error))));
            }
            Ok(Value::Undefined)
        });
        let state = watcher.clone();
        realm.method(&object, "close", 0, move |realm, _, _| {
            let timer = {
                let mut watcher = state.borrow_mut();
                watcher.active = false;
                watcher.closed = true;
                watcher.referenced.set(false);
                watcher.timer.take()
            };
            if let Some(timer) = timer {
                realm.clear(&Value::Object(timer));
            }
            Ok(Value::Undefined)
        });
        for (name, referenced) in [("ref", true), ("unref", false)] {
            let state = watcher.clone();
            realm.method(&object, name, 0, move |_, this, _| {
                if !state.borrow().closed {
                    state.borrow().referenced.set(referenced);
                }
                Ok(this)
            });
        }
        let state = watcher.clone();
        realm.method(&object, "hasRef", 0, move |_, _, _| {
            Ok(Value::Boolean(state.borrow().referenced.get()))
        });
        let id = binding.next_async_id.fetch_add(1, Ordering::Relaxed);
        realm.method(&object, "getAsyncId", 0, move |_, _, _| {
            Ok(Value::Number(id as f64))
        });
        Ok(Value::Undefined)
    })
}

#[cfg(test)]
mod tests;
