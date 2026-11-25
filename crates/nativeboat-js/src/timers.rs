//! Native timer storage and dispatch. Idle polls create no JavaScript arrays,
//! iterators or sorted entry objects; callbacks remain traced Boa values.

use super::{Queue, report_error, run_microtasks};
use boa_engine::object::{FunctionObjectBuilder, builtins::JsArray};
use boa_engine::{
    Context, Finalize, JsArgs, JsNativeError, JsObject, JsResult, JsString, JsValue,
    NativeFunction, Trace,
};
use boa_gc::{Gc, GcRefCell};
use std::{collections::HashMap, rc::Rc, time::Instant};

#[derive(Clone, Trace, Finalize)]
struct Timer {
    callback: JsObject,
    args: Vec<JsValue>,
    at: f64,
    delay: f64,
    repeat: bool,
}

#[derive(Default, Trace, Finalize)]
struct State {
    next: u64,
    timers: HashMap<u64, Timer>,
}

#[derive(Clone, Trace, Finalize)]
struct Timers {
    state: Gc<GcRefCell<State>>,
    #[unsafe_ignore_trace]
    started: Instant,
}

impl Timers {
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    fn due(&self, now: f64) -> Vec<u64> {
        let state = self.state.borrow();
        let mut due: Vec<_> = state
            .timers
            .iter()
            .filter(|(_, timer)| timer.at <= now)
            .map(|(id, timer)| (*id, timer.at))
            .collect();
        due.sort_unstable_by(|(a, left), (b, right)| left.total_cmp(right).then(a.cmp(b)));
        due.into_iter().map(|(id, _)| id).collect()
    }

    fn take(&self, id: u64, now: f64) -> Option<Timer> {
        let mut state = self.state.borrow_mut();
        let timer = state.timers.get_mut(&id)?;
        let result = timer.clone();
        if timer.repeat {
            timer.at = now + timer.delay;
        } else {
            state.timers.remove(&id);
        }
        Some(result)
    }
}

fn id(args: &[JsValue]) -> Option<u64> {
    args.get_or_undefined(0)
        .as_number()
        .filter(|value| value.is_finite() && *value > 0.0 && value.fract() == 0.0)
        .map(|value| value as u64)
}

fn schedule(
    _: &JsValue,
    args: &[JsValue],
    timers: &Timers,
    context: &mut Context,
) -> JsResult<JsValue> {
    let callback = args.get_or_undefined(0).as_callable().ok_or_else(|| {
        JsNativeError::typ().with_message("The \"callback\" argument must be of type function")
    })?;
    let delay = args.get_or_undefined(1).to_number(context)?;
    let delay = if delay.is_nan() { 0.0 } else { delay.max(0.0) };
    let array = args
        .get_or_undefined(2)
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("timer arguments must be an array"))?;
    let array = JsArray::from_object(array)?;
    let arguments = (0..array.length(context)?)
        .map(|index| array.get(index, context))
        .collect::<JsResult<Vec<_>>>()?;
    let repeat = args.get_or_undefined(3).to_boolean();
    let timer = Timer {
        callback,
        args: arguments,
        at: timers.now() + delay,
        delay: if repeat { delay.max(1.0) } else { delay },
        repeat,
    };
    let mut state = timers.state.borrow_mut();
    state.next += 1;
    let id = state.next;
    state.timers.insert(id, timer);
    Ok(JsValue::from(id as f64))
}

fn cancel(_: &JsValue, args: &[JsValue], timers: &Timers, _: &mut Context) -> JsResult<JsValue> {
    if let Some(id) = id(args) {
        timers.state.borrow_mut().timers.remove(&id);
    }
    Ok(JsValue::undefined())
}

fn refresh(_: &JsValue, args: &[JsValue], timers: &Timers, _: &mut Context) -> JsResult<JsValue> {
    if let Some(id) = id(args)
        && let Some(timer) = timers.state.borrow_mut().timers.get_mut(&id)
    {
        timer.at = timers.now() + timer.delay;
    }
    Ok(JsValue::undefined())
}

fn next(_: &JsValue, _: &[JsValue], timers: &Timers, _: &mut Context) -> JsResult<JsValue> {
    let soonest = timers
        .state
        .borrow()
        .timers
        .values()
        .map(|timer| timer.at)
        .fold(f64::INFINITY, f64::min);
    Ok(JsValue::from(if soonest == f64::INFINITY {
        -1.0
    } else {
        (soonest - timers.now()).max(0.0)
    }))
}

#[derive(Clone, Trace, Finalize)]
struct Driver {
    timers: Timers,
    host: JsObject,
    // As in Shared/Boa's executors, queued jobs/promises are Rust roots.
    #[unsafe_ignore_trace]
    queue: Rc<Queue>,
}

fn run(_: &JsValue, _: &[JsValue], driver: &Driver, context: &mut Context) -> JsResult<JsValue> {
    let now = driver.timers.now();
    // Snapshot due ids once: newly scheduled work waits for the next poll.
    // Cancellation is checked before each callback, including after microtasks.
    for id in driver.timers.due(now) {
        let Some(timer) = driver.timers.take(id, now) else {
            continue;
        };
        if let Err(error) =
            timer
                .callback
                .call(&context.global_object().into(), &timer.args, context)
        {
            if error.as_engine().is_some() {
                return Err(error);
            }
            report_error(context, &driver.host, error);
        }
        run_microtasks(context, &driver.host, &driver.queue);
    }
    Ok(JsValue::undefined())
}

pub(super) fn install(host: &JsObject, started: Instant, queue: Rc<Queue>, context: &mut Context) {
    let timers = Timers {
        state: Gc::new(GcRefCell::new(State::default())),
        started,
    };
    let driver = Driver {
        timers: timers.clone(),
        host: host.clone(),
        queue,
    };
    for (name, function) in [
        (
            "timerSchedule",
            NativeFunction::from_copy_closure_with_captures(schedule, timers.clone()),
        ),
        (
            "timerCancel",
            NativeFunction::from_copy_closure_with_captures(cancel, timers.clone()),
        ),
        (
            "timerRefresh",
            NativeFunction::from_copy_closure_with_captures(refresh, timers.clone()),
        ),
        (
            "timerNext",
            NativeFunction::from_copy_closure_with_captures(next, timers),
        ),
        (
            "timerRun",
            NativeFunction::from_copy_closure_with_captures(run, driver),
        ),
    ] {
        let function = FunctionObjectBuilder::new(context.realm(), function)
            .name(JsString::from(name))
            .build();
        host.set(JsString::from(name), function, false, context)
            .expect("the host object is extensible");
    }
}

#[cfg(test)]
mod tests;
