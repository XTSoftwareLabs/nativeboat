//! Node's process events. When the Node.js library is linked, `process` is
//! an EventEmitter as Node's bootstrap makes it, and a program's lifecycle
//! follows Node's: uncaught exceptions and unhandled rejections are offered
//! to `uncaughtException` and `unhandledRejection` listeners, an empty event
//! loop emits `beforeExit`, and the program ends with `exit`.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Key, Object, Realm, Thrown, Value, ops, platform};

/// Makes `process` an EventEmitter, as Node's bootstrap does, with Node's
/// own listeners: signal bookkeeping and the default `warning` printer.
/// Library modules read `process` while `events` loads; it becomes an
/// emitter once `events` has loaded.
pub(crate) fn emitter(realm: &mut Realm, process: &Object) -> Result<(), Thrown> {
    let events = crate::modules::load(realm, "node:events")?;
    if realm.process_emitter || !crate::array_builtin::is_callable(&events) {
        return Ok(());
    }
    realm.process_emitter = true;
    let prototype = ops::get_named(realm, &events, "prototype")?;
    let own = process.data().prototype.clone();
    if let Some(own) = own {
        own.set_prototype(prototype.as_object().cloned());
    }
    let target = Value::Object(process.clone());
    ops::call(realm, &events, target.clone(), &[])?;
    let ignore = |_: &mut Realm, _: Value, _: &[Value]| Ok(Value::Undefined);
    // Node tracks `multipleResolves` listeners, then signal listeners.
    let listeners = [
        ("newListener", realm.function("", 1, false, ignore)),
        ("removeListener", realm.function("", 1, false, ignore)),
        (
            "newListener",
            realm.function("startListeningIfSignal", 1, false, ignore),
        ),
        (
            "removeListener",
            realm.function("stopListeningIfSignal", 1, false, ignore),
        ),
        (
            "warning",
            realm.function("onWarning", 1, false, |realm, _, args| {
                let warning = args.first().cloned().unwrap_or_default();
                crate::process_builtin::on_warning(realm, &warning)?;
                Ok(Value::Undefined)
            }),
        ),
    ];
    let on = ops::get_named(realm, &target, "on")?;
    for (name, listener) in listeners {
        let args = [Value::str(name), Value::Object(listener)];
        ops::call(realm, &on, target.clone(), &args)?;
    }
    Ok(())
}

/// `process.emit(name, ...args)`: whether a listener ran, or `None` when
/// `process` is not an EventEmitter (or was never read, so nothing listens).
pub(crate) fn emit(realm: &mut Realm, name: &str, args: &[Value]) -> Result<Option<bool>, Thrown> {
    let Some(process) = realm.process.clone() else {
        return Ok(None);
    };
    let process = Value::Object(process);
    let emit = ops::get_named(realm, &process, "emit")?;
    if !crate::array_builtin::is_callable(&emit) {
        return Ok(None);
    }
    let mut arguments = vec![Value::str(name)];
    arguments.extend_from_slice(args);
    let result = ops::call(realm, &emit, process, &arguments)?;
    Ok(Some(ops::to_boolean(&result)))
}

/// `process._exiting`: whether the `exit` event has been emitted.
fn exiting(realm: &mut Realm) -> bool {
    let Some(process) = realm.process.clone() else {
        return false;
    };
    ops::get_named(realm, &Value::Object(process), "_exiting")
        .is_ok_and(|exiting| ops::to_boolean(&exiting))
}

/// Emits `exit` with the exit code, once.
pub(crate) fn exit(realm: &mut Realm) -> Result<(), Thrown> {
    let Some(process) = realm.process.clone().filter(|_| !exiting(realm)) else {
        return Ok(());
    };
    let process = Value::Object(process);
    ops::set(
        realm,
        &process,
        &Key::name("_exiting"),
        Value::Boolean(true),
    )?;
    let code = Value::Number(f64::from(realm.exit_status()));
    emit(realm, "exit", &[code]).map(drop)
}

/// Node's `process._fatalException`: offers an uncaught exception to the
/// `uncaughtExceptionMonitor` and `uncaughtException` listeners. Without a
/// listener the program ends: `exit` listeners run with code 1, the error
/// is reported, and the realm records the exit status, which is 7 when a
/// listener throws. The exception is returned when the program ends.
pub(crate) fn fatal_exception(
    realm: &mut Realm,
    thrown: Thrown,
    origin: &str,
) -> Result<(), Thrown> {
    let args = [thrown.0.clone(), Value::str(origin)];
    let handled = emit(realm, "uncaughtExceptionMonitor", &args)
        .and_then(|_| emit(realm, "uncaughtException", &args));
    let status = match handled {
        Ok(Some(true)) => return Ok(()),
        Ok(_) => {
            if !exiting(realm) {
                realm.exit_code = Value::Number(1.0);
                // Node ignores errors in `exit` listeners here.
                let _ = exit(realm);
            }
            report(&mut *realm, &thrown.0);
            match realm.exit_code {
                Value::Number(code) => ops::wrap32(code) as i32,
                _ => 1,
            }
        }
        Err(error) => {
            report(&mut *realm, &error.0);
            7
        }
    };
    realm.event_loop.fatal = Some(status);
    Err(thrown)
}

fn report(realm: &mut Realm, error: &Value) {
    let report = realm.uncaught(error);
    platform::write_stderr(&report);
}

impl Realm {
    /// Runs a program's main module and then its event loop as Node does,
    /// returning the process exit code.
    pub fn run_main<T>(&mut self, main: impl FnOnce(&mut Realm) -> Result<T, Thrown>) -> i32 {
        let result = main(self).map(drop);
        self.finish_main(result)
    }

    /// Runs the event loop after the main module, which completed with
    /// `result`, through `beforeExit` and `exit`.
    fn finish_main(&mut self, mut result: Result<(), Thrown>) -> i32 {
        loop {
            loop {
                if let Err(thrown) = result
                    && let Some(status) = self.fatal(thrown)
                {
                    return status;
                }
                result = self.run_event_loop();
                if result.is_ok() {
                    break;
                }
            }
            // `beforeExit` listeners may schedule more work.
            match self.before_exit() {
                Ok(true) => result = Ok(()),
                Ok(false) => break,
                Err(thrown) => {
                    if let Some(status) = self.fatal(thrown) {
                        return status;
                    }
                    break;
                }
            }
        }
        if let Err(thrown) = exit(self)
            && let Some(status) = self.fatal(thrown)
        {
            return status;
        }
        self.exit_status()
    }

    /// Evaluates a context whose event loop a native host loop drives: its
    /// entry module and the jobs that follow it. Uncaught exceptions are
    /// offered to listeners as in `run_main`; the status is returned when one
    /// ends the program.
    pub fn start_host<T>(
        &mut self,
        main: impl FnOnce(&mut Realm) -> Result<T, Thrown>,
    ) -> Option<i32> {
        let mut result = main(self).map(drop);
        loop {
            if let Err(thrown) = result
                && let Some(status) = self.fatal(thrown)
            {
                return Some(status);
            }
            result = self.run_jobs();
            if result.is_ok() {
                return None;
            }
        }
    }

    /// Runs the work due in a host-driven context, handling uncaught
    /// exceptions; the exit status when one ends the program.
    pub fn step_host(&mut self) -> Result<crate::HostWait, i32> {
        for thrown in core::mem::take(&mut self.host_errors) {
            if let Some(status) = self.fatal(thrown) {
                return Err(status);
            }
        }
        loop {
            match self.run_due() {
                Ok(wait) => {
                    // Menu changes made while the step ran reach the native
                    // menus once, before the host waits for input.
                    if let Err(thrown) = crate::electron_menu::flush_menus(self)
                        && let Some(status) = self.fatal(thrown)
                    {
                        return Err(status);
                    }
                    // A page's workers run after it.
                    #[cfg(feature = "dom")]
                    let wait = crate::worker::step(self, wait);
                    return Ok(wait);
                }
                Err(thrown) => {
                    if let Some(status) = self.fatal(thrown) {
                        return Err(status);
                    }
                }
            }
        }
    }

    /// Runs a started context's event loop to completion as `run_main` does
    /// after its main module, through `beforeExit` and `exit`.
    pub fn run_to_completion(&mut self) -> i32 {
        self.finish_main(Ok(()))
    }

    /// Ends a host-driven context as Node ends a program: `exit` listeners
    /// run once, then the exit status.
    pub fn finish_host(&mut self) -> i32 {
        if let Err(thrown) = exit(self)
            && let Some(status) = self.fatal(thrown)
        {
            return status;
        }
        crate::trace::exit(&format!("exit status {}", self.exit_status()));
        self.exit_status()
    }

    /// Queues an exception thrown outside a job, which the next host step
    /// handles as uncaught.
    pub fn queue_host_error(&mut self, thrown: Thrown) {
        self.host_errors.push(thrown);
    }

    /// Handles an uncaught exception, returning the exit status when it
    /// ends the program.
    fn fatal(&mut self, thrown: Thrown) -> Option<i32> {
        if crate::trace::enabled() {
            let text = match &thrown.0 {
                Value::Object(_) => crate::builtins::error_to_string(self, thrown.0.clone(), &[]),
                value => Ok(value.clone()),
            };
            let text = match text.and_then(|text| ops::to_string(self, &text)) {
                Ok(text) => text.to_rust_lossy(),
                Err(_) => "a value that cannot be converted to a string".into(),
            };
            #[cfg(feature = "dom")]
            let renderer = self.web_host.is_some();
            #[cfg(not(feature = "dom"))]
            let renderer = false;
            let role = match (renderer, self.electron_window) {
                (true, _) => "renderer",
                (false, 0) => "main",
                (false, _) => "preload",
            };
            crate::trace::exit(&format!("uncaught exception in the {role} realm: {text}"));
        }
        // A renderer page reports its errors as a browser does, with `error`
        // events and the console, and keeps running.
        #[cfg(feature = "dom")]
        if self.web_host.is_some() {
            crate::web::uncaught(self, thrown);
            return None;
        }
        // A preload script runs in a renderer too: Electron offers the
        // exception to `uncaughtException` listeners and otherwise reports it
        // on the console, and the page keeps running.
        if self.electron_window != 0 {
            let args = [thrown.0.clone(), Value::str("uncaughtException")];
            let handled = emit(self, "uncaughtExceptionMonitor", &args)
                .and_then(|_| emit(self, "uncaughtException", &args));
            match handled {
                Ok(Some(true)) => {}
                Ok(_) => report(self, &thrown.0),
                Err(error) => report(self, &error.0),
            }
            return None;
        }
        if self.event_loop.fatal.is_none() {
            let _ = fatal_exception(self, thrown, "uncaughtException");
        }
        self.event_loop.fatal
    }

    /// Emits `beforeExit` unless the program is exiting, and runs the work
    /// its listeners queue. Returns whether the event loop is alive again.
    fn before_exit(&mut self) -> Result<bool, Thrown> {
        if exiting(self) {
            return Ok(false);
        }
        let code = Value::Number(f64::from(self.exit_status()));
        emit(self, "beforeExit", &[code])?;
        self.run_jobs()?;
        Ok(self.alive())
    }
}

#[cfg(test)]
mod tests;
