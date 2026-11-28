//! The job queue and event loop that run after module evaluation, as Node
//! 22 runs them: `process.nextTick` callbacks, then promise jobs and
//! `queueMicrotask` callbacks, drain after every task; timers run in due order, then immediates; the loop sleeps
//! until the next timer and ends when no referenced timer or immediate
//! remains. Rejections still unhandled after the jobs drain go to
//! `unhandledRejection` listeners, or become uncaught exceptions as with
//! Node's default `--unhandled-rejections=throw`. After an uncaught exception
//! is handled the loop resumes where it stopped (see `process_events`).
//! Timers and immediates return Node's handle objects.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Kind, Object, Property};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::value::{Completion, Thrown, Value};
use crate::work_queue::{Signal, Work};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};

/// A queued job: a promise job or a microtask callback.
pub(crate) type Job = Box<dyn FnOnce(&mut Realm) -> Result<(), Thrown>>;
type IoCompletion<T> = Box<dyn FnOnce(&mut Realm, T) -> Result<(), Thrown>>;

/// A pending native operation and the handle state that keeps it alive.
struct PendingIo {
    poll: Box<dyn FnMut() -> Option<Job>>,
    /// Ordinary I/O is always referenced; ref-able handles supply live state.
    referenced: Option<Rc<Cell<bool>>>,
}

impl PendingIo {
    fn referenced(&self) -> bool {
        self.referenced.as_ref().is_none_or(|state| state.get())
    }
}

/// A job that runs in an async context frame. Work queued with no frame
/// runs as is: jobs start from none, as the loop runs them.
fn in_frame(context: Value, job: Job) -> Job {
    if matches!(context, Value::Undefined) {
        return job;
    }
    Box::new(move |realm| {
        let previous = core::mem::replace(&mut realm.async_context, context);
        let result = job(realm);
        realm.async_context = previous;
        result
    })
}

/// Node's largest timer delay; longer or invalid delays become 1 ms.
const TIMEOUT_MAX: f64 = 2_147_483_647.0;

/// Callback storage follows the handle's lifetime, so a completed timeout can
/// be refreshed without retaining every completed handle in the event loop.
#[derive(Debug, Clone)]
struct Callback {
    function: Value,
    args: Vec<Value>,
}

/// Native Timeout/Immediate state, retained by its handle.
#[derive(Debug)]
pub struct Timer {
    pub(crate) id: Cell<u64>,
    callback: RefCell<Option<Callback>>,
    delay: Option<f64>,
    repeat: bool,
    referenced: Rc<Cell<bool>>,
    /// The async context frame the timer was created in.
    context: Value,
    running: Cell<bool>,
}

/// A pending timer or immediate. Timer keys include insertion order, since
/// refreshing a timer appends it after other timers with the same deadline.
struct Task {
    handle: Object,
    state: Rc<Timer>,
    due: Option<(u64, u64)>,
}

/// The queues of one realm.
pub(crate) struct EventLoop {
    /// `process.nextTick` callbacks, which run before promise jobs.
    ticks: VecDeque<Job>,
    /// Next-tick callbacks a page's code queued in an Electron renderer's
    /// Node environment, which run on its next turn, after the page has
    /// rendered.
    deferred_ticks: VecDeque<Job>,
    /// Set while a page's checkpoint runs this realm's jobs: the ticks they
    /// queue are deferred.
    pub(crate) deferring_ticks: bool,
    jobs: VecDeque<Job>,
    tasks: BTreeMap<u64, Task>,
    /// Timers by due time, then creation order.
    timers: BTreeMap<(u64, u64), u64>,
    immediates: VecDeque<u64>,
    next_id: u64,
    next_order: u64,
    io: Vec<PendingIo>,
    io_signal: Option<Arc<Signal>>,
    pub(crate) io_create: fn() -> Result<Arc<Signal>, u32>,
    /// Rejected promises without handlers, in rejection order.
    pub(crate) unhandled: Vec<(Object, Value)>,
    /// Set when a next-tick callback throws: the rest of the queue waits
    /// until after the next task, as in Node.
    skip_drain: bool,
    /// The exit status once an uncaught exception ends the program.
    pub(crate) fatal: Option<i32>,
    /// The monotonic clock in milliseconds and the way to wait.
    pub(crate) clock: fn() -> f64,
    pub(crate) sleep: fn(f64),
}

impl Default for EventLoop {
    fn default() -> EventLoop {
        EventLoop {
            ticks: VecDeque::new(),
            deferred_ticks: VecDeque::new(),
            deferring_ticks: false,
            jobs: VecDeque::new(),
            tasks: BTreeMap::new(),
            timers: BTreeMap::new(),
            immediates: VecDeque::new(),
            next_id: 0,
            next_order: 0,
            io: Vec::new(),
            io_signal: None,
            io_create: Signal::new,
            unhandled: Vec::new(),
            skip_drain: false,
            fatal: None,
            clock: crate::platform::now,
            sleep: crate::platform::sleep,
        }
    }
}

impl core::fmt::Debug for EventLoop {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "EventLoop({} jobs, {} tasks)",
            self.jobs.len(),
            self.tasks.len()
        )
    }
}

/// What a host-driven realm waits for after a turn of its event loop.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HostWait {
    /// Milliseconds until the next timer is due; 0 when work is ready now.
    pub delay: Option<u32>,
    /// A Windows event handle signaled when native I/O completes.
    pub signal: Option<usize>,
}

impl HostWait {
    /// The earliest of two waits, with both I/O signals.
    pub fn merge(self, other: HostWait, signals: &mut Vec<usize>) -> HostWait {
        signals.extend(self.signal);
        signals.extend(other.signal);
        let delay = match (self.delay, other.delay) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (left, right) => left.or(right),
        };
        HostWait {
            delay,
            signal: None,
        }
    }
}

fn micros(milliseconds: f64) -> u64 {
    (milliseconds * 1000.0) as u64
}

impl Realm {
    /// Runs owned native work on the Windows pool, then completes it on this
    /// realm's thread. Source objects cannot cross into the worker: its inputs
    /// and result must be Send. A pending request keeps the event loop alive.
    pub fn submit_io<T: Send + 'static>(
        &mut self,
        work: impl FnOnce() -> T + Send + 'static,
        complete: impl FnOnce(&mut Realm, T) -> Result<(), Thrown> + 'static,
    ) -> Result<(), u32> {
        self.submit_io_boxed(Box::new(work), Box::new(complete), None)
    }

    /// Submits native work owned by a ref-able handle such as an FSWatcher.
    /// The live reference state controls whether pending work keeps Node alive.
    pub fn submit_io_with_ref<T: Send + 'static>(
        &mut self,
        work: impl FnOnce() -> T + Send + 'static,
        referenced: Rc<Cell<bool>>,
        complete: impl FnOnce(&mut Realm, T) -> Result<(), Thrown> + 'static,
    ) -> Result<(), u32> {
        self.submit_io_boxed(Box::new(work), Box::new(complete), Some(referenced))
    }

    // Keep queue construction independent of the caller's closure types. A
    // rejected submission must not create a unique, unreachable completion.
    fn submit_io_boxed<T: Send + 'static>(
        &mut self,
        work: Box<dyn FnOnce() -> T + Send>,
        complete: IoCompletion<T>,
        referenced: Option<Rc<Cell<bool>>>,
    ) -> Result<(), u32> {
        let signal = match &self.event_loop.io_signal {
            Some(signal) => signal.clone(),
            None => {
                let signal = (self.event_loop.io_create)()?;
                self.event_loop.io_signal = Some(signal.clone());
                signal
            }
        };
        let mut work = Work::submit(signal, work)?;
        let complete = self.in_context(complete);
        let mut complete = Some(complete);
        self.event_loop.io.push(PendingIo {
            poll: Box::new(move || {
                let result = work.take()?;
                let complete = complete.take().expect("one completion per native request");
                Some(Box::new(move |realm| complete(realm, result)))
            }),
            referenced,
        });
        Ok(())
    }

    fn run_io(&mut self) -> Result<(), Thrown> {
        // Requests submitted by a completion belong to a later poll phase.
        // Keep that phase bounded so chained work cannot starve immediates.
        let mut remaining = self.event_loop.io.len();
        while remaining != 0 {
            let ready = self
                .event_loop
                .io
                .iter_mut()
                .take(remaining)
                .enumerate()
                .find_map(|(index, pending)| (pending.poll)().map(|job| (index, job)));
            let Some((index, job)) = ready else {
                return Ok(());
            };
            remaining -= 1;
            drop(self.event_loop.io.remove(index));
            job(self)?;
            self.run_jobs()?;
        }
        Ok(())
    }

    /// `HostEnqueuePromiseJob`: runs `job` once the current task and the
    /// jobs before it finish, in the async context it was queued in.
    pub(crate) fn enqueue(&mut self, job: Job) {
        self.enqueue_in(self.async_context.clone(), job);
    }

    /// Queues a promise job that runs in `context`, the async context frame
    /// of the `then` or `await` that registered it.
    pub(crate) fn enqueue_in(&mut self, context: Value, job: Job) {
        let job = in_frame(context, job);
        self.event_loop.jobs.push_back(job);
    }

    /// Queues a `process.nextTick` callback.
    pub(crate) fn enqueue_tick(&mut self, job: Job) {
        let job = in_frame(self.async_context.clone(), job);
        self.event_loop.ticks.push_back(job);
    }

    /// An I/O completion that runs in the current async context frame.
    fn in_context<T: 'static>(&self, complete: IoCompletion<T>) -> IoCompletion<T> {
        if matches!(self.async_context, Value::Undefined) {
            return complete;
        }
        let context = self.async_context.clone();
        Box::new(move |realm, result| {
            let previous = core::mem::replace(&mut realm.async_context, context);
            let completed = complete(realm, result);
            realm.async_context = previous;
            completed
        })
    }

    /// Runs queued work until none remains, as Node does after each task:
    /// all next-tick callbacks, then all promise jobs, again while new ticks
    /// were queued. Then offers the rejections still unhandled to
    /// `unhandledRejection` listeners, or treats them as uncaught
    /// exceptions, and drains again.
    pub fn run_jobs(&mut self) -> Result<(), Thrown> {
        loop {
            loop {
                if self.event_loop.deferring_ticks {
                    let ticks = core::mem::take(&mut self.event_loop.ticks);
                    self.event_loop.deferred_ticks.extend(ticks);
                }
                while let Some(tick) = self.event_loop.ticks.pop_front() {
                    tick(self).inspect_err(|_| self.event_loop.skip_drain = true)?;
                }
                // Like V8, a job's uncaught exception does not stop the others.
                while let Some(job) = self.event_loop.jobs.pop_front() {
                    if let Err(thrown) = job(self) {
                        crate::process_events::fatal_exception(self, thrown, "uncaughtException")?;
                    }
                }
                if self.event_loop.ticks.is_empty() {
                    break;
                }
            }
            // Each drain ends a job: WeakRef targets may be collected again.
            self.clear_kept();
            if self.event_loop.unhandled.is_empty() {
                return Ok(());
            }
            // The rejections pending now; later ones wait for the next drain.
            for (promise, reason) in core::mem::take(&mut self.event_loop.unhandled) {
                let args = [reason.clone(), Value::Object(promise)];
                if crate::process_events::emit(self, "unhandledRejection", &args)? != Some(true) {
                    let error = self.unhandled_rejection(reason);
                    crate::process_events::fatal_exception(self, error, "unhandledRejection")?;
                }
            }
        }
    }

    /// Runs the next-tick callbacks a page's code queued in this Electron
    /// renderer's Node environment, ahead of those queued since, as Electron
    /// runs them on its next turn after the page has rendered. An uncaught
    /// exception reaches the host at the realm's next step.
    pub fn run_deferred_ticks(&mut self) {
        if self.event_loop.deferred_ticks.is_empty() {
            return;
        }
        let mut deferred = core::mem::take(&mut self.event_loop.deferred_ticks);
        deferred.append(&mut self.event_loop.ticks);
        self.event_loop.ticks = deferred;
        if let Err(thrown) = self.run_jobs() {
            self.host_errors.push(thrown);
        }
    }

    /// Whether next-tick callbacks wait for the page to render.
    pub fn has_deferred_ticks(&self) -> bool {
        !self.event_loop.deferred_ticks.is_empty()
    }

    /// Whether ticks or promise jobs are waiting to run, such as those another
    /// realm queued here after this one's last step.
    pub fn has_queued_jobs(&self) -> bool {
        #[cfg(feature = "dom")]
        if crate::worker::has_queued_jobs(self) {
            return true;
        }
        !self.event_loop.ticks.is_empty() || !self.event_loop.jobs.is_empty()
    }

    /// Milliseconds until this realm has work to run: none for queued jobs
    /// and immediates, else until its earliest timer. Other realms may have
    /// scheduled work here since its last step.
    pub fn next_wake(&self) -> Option<u32> {
        if self.has_queued_jobs() || !self.event_loop.immediates.is_empty() {
            return Some(0);
        }
        let now = micros((self.event_loop.clock)());
        let own = self.event_loop.timers.keys().next().map(|(due, _)| {
            crate::float::ceil(due.saturating_sub(now) as f64 / 1000.0).min(f64::from(u32::MAX - 1))
                as u32
        });
        #[cfg(feature = "dom")]
        let own = match (own, crate::worker::next_wake(self)) {
            (Some(own), Some(workers)) => Some(own.min(workers)),
            (own, workers) => own.or(workers),
        };
        own
    }

    /// Whether work is due within some milliseconds: queued jobs,
    /// immediates, or timers set to fire about then, as the timers a page
    /// sets without a delay while it starts.
    pub fn has_imminent_work(&self, within_ms: u32) -> bool {
        if self.has_queued_jobs() || !self.event_loop.immediates.is_empty() {
            return true;
        }
        let soon = micros((self.event_loop.clock)()) + u64::from(within_ms) * 1000;
        self.event_loop
            .timers
            .keys()
            .next()
            .is_some_and(|(due, _)| *due <= soon)
    }

    /// Whether queued jobs or referenced timers, immediates or I/O keep the
    /// program running.
    pub fn keeps_alive(&self) -> bool {
        self.has_queued_jobs() || self.alive()
    }

    /// Whether a referenced timer or immediate keeps the program running.
    pub(crate) fn alive(&self) -> bool {
        self.event_loop.io.iter().any(PendingIo::referenced)
            || self
                .event_loop
                .tasks
                .values()
                .any(|task| task.state.referenced.get())
    }

    /// The error Node reports for an unhandled rejection: the reason itself
    /// when it is like an error, an object with its own `stack` as a
    /// `DOMException` has, otherwise an explanation naming it.
    fn unhandled_rejection(&mut self, reason: Value) -> Thrown {
        if reason.as_object().is_some_and(|object| {
            matches!(object.data().kind, Kind::Error)
                || object.own_property(&Key::name("stack")).is_some()
        }) {
            return Thrown(reason);
        }
        let text = crate::inspect::inspect(self, &reason, crate::inspect::DEPTH)
            .unwrap_or_else(|_| "<unprintable>".to_owned());
        let error = self.error(
            ErrorKind::Error,
            &format!(
                "This error originated either by throwing inside of an async function without a catch block, or by rejecting a promise which was not handled with .catch(). The promise rejected with the reason \"{text}\"."
            ),
        );
        error.define(
            Key::name("name"),
            Property::hidden(Value::str("UnhandledPromiseRejection")),
        );
        Thrown(Value::Object(error))
    }

    /// Runs jobs, timers and immediates until nothing keeps the program
    /// alive. A callback's uncaught exception ends the loop.
    /// One turn of the event loop for a realm driven by a native host loop:
    /// runs the jobs, timers, completed I/O and immediates that are due,
    /// without waiting, and reports what the realm waits for next.
    pub fn run_due(&mut self) -> Result<HostWait, Thrown> {
        if !core::mem::take(&mut self.event_loop.skip_drain) {
            self.run_jobs()?;
        }
        self.finalize()?;
        let now = micros((self.event_loop.clock)());
        let due: Vec<u64> = self
            .event_loop
            .timers
            .iter()
            .take_while(|((due, _), _)| *due <= now)
            .map(|(_, id)| *id)
            .collect();
        for id in due {
            self.run_task(id)?;
        }
        self.run_io()?;
        let mut immediates: VecDeque<u64> = self.event_loop.immediates.drain(..).collect();
        while let Some(id) = immediates.pop_front() {
            if let Err(thrown) = self.run_task(id) {
                while let Some(id) = immediates.pop_back() {
                    self.event_loop.immediates.push_front(id);
                }
                return Err(thrown);
            }
        }
        let ready = !self.event_loop.immediates.is_empty()
            || !self.event_loop.ticks.is_empty()
            || !self.event_loop.jobs.is_empty();
        let now = micros((self.event_loop.clock)());
        let delay = if ready {
            Some(0)
        } else {
            self.event_loop.timers.keys().next().map(|(due, _)| {
                crate::float::ceil(due.saturating_sub(now) as f64 / 1000.0)
                    .min(f64::from(u32::MAX - 1)) as u32
            })
        };
        let signal = if self.event_loop.io.is_empty() {
            None
        } else {
            self.event_loop
                .io_signal
                .as_ref()
                .map(|signal| signal.raw())
        };
        Ok(HostWait { delay, signal })
    }

    pub fn run_event_loop(&mut self) -> Result<(), Thrown> {
        if !core::mem::take(&mut self.event_loop.skip_drain) {
            self.run_jobs()?;
        }
        loop {
            self.finalize()?;
            if !self.alive() {
                return Ok(());
            }
            let now = micros((self.event_loop.clock)());
            let due: Vec<u64> = self
                .event_loop
                .timers
                .iter()
                .take_while(|((due, _), _)| *due <= now)
                .map(|(_, id)| *id)
                .collect();
            for id in due {
                self.run_task(id)?;
            }
            self.run_io()?;
            // Immediates queued by these run on the next turn; after an
            // uncaught exception the rest of this turn's run when it resumes.
            let mut immediates: VecDeque<u64> = self.event_loop.immediates.drain(..).collect();
            while let Some(id) = immediates.pop_front() {
                if let Err(thrown) = self.run_task(id) {
                    while let Some(id) = immediates.pop_back() {
                        self.event_loop.immediates.push_front(id);
                    }
                    return Err(thrown);
                }
            }
            if !self.alive() {
                return Ok(());
            }
            if self.event_loop.immediates.is_empty() {
                let next = self.event_loop.timers.keys().next().map(|(due, _)| *due);
                let now = micros((self.event_loop.clock)());
                if !self.event_loop.io.is_empty() {
                    let delay = next.map_or(u32::MAX, |next| {
                        crate::float::ceil(next.saturating_sub(now) as f64 / 1000.0)
                            .min(f64::from(u32::MAX - 1)) as u32
                    });
                    if let Err(error) = self
                        .event_loop
                        .io_signal
                        .as_ref()
                        .expect("submitted work has a signal")
                        .wait(delay)
                    {
                        return Err(self.throw(
                            ErrorKind::Error,
                            &format!("Native I/O completion wait failed: Windows error {error}"),
                        ));
                    }
                } else if let Some(next) = next.filter(|next| *next > now) {
                    (self.event_loop.sleep)((next - now) as f64 / 1000.0);
                }
            }
        }
    }

    /// Inserts a timeout with a fresh deadline and insertion order.
    fn activate(&mut self, handle: Object, state: Rc<Timer>) {
        let id = state.id.get();
        self.remove_task(id);
        self.event_loop.next_order += 1;
        let due = state.delay.map(|delay| {
            (
                micros((self.event_loop.clock)() + delay),
                self.event_loop.next_order,
            )
        });
        if let Some(key) = due {
            self.event_loop.timers.insert(key, id);
        } else {
            self.event_loop.immediates.push_back(id);
        }
        self.event_loop
            .tasks
            .insert(id, Task { handle, state, due });
    }

    fn remove_task(&mut self, id: u64) -> Option<Task> {
        let task = self.event_loop.tasks.remove(&id)?;
        match task.due {
            Some(key) => {
                self.event_loop.timers.remove(&key);
            }
            None => self.event_loop.immediates.retain(|queued| *queued != id),
        }
        Some(task)
    }

    /// A callback may clear or refresh another timer already selected for
    /// this phase. Check its current deadline again before removing it.
    fn run_task(&mut self, id: u64) -> Result<(), Thrown> {
        if self.event_loop.tasks.get(&id).is_some_and(|task| {
            task.due
                .is_some_and(|(due, _)| due > micros((self.event_loop.clock)()))
        }) {
            return Ok(());
        }
        let Some(task) = self.remove_task(id) else {
            return Ok(());
        };
        let state = task.state;
        let callback = state
            .callback
            .borrow()
            .clone()
            .expect("a scheduled callback");
        if state.delay.is_none() {
            state.referenced.set(false);
            state.callback.borrow_mut().take();
        }
        state.running.set(true);
        if state.repeat {
            self.activate(task.handle.clone(), state.clone());
        }
        let previous = core::mem::replace(&mut self.async_context, state.context.clone());
        let result = ops::call(
            self,
            &callback.function,
            Value::Object(task.handle),
            &callback.args,
        );
        self.async_context = previous;
        state.running.set(false);
        result?;
        self.run_jobs()
    }

    fn schedule(&mut self, args: &[Value], timeout: bool, repeat: bool) -> Completion {
        let callback = args.first().cloned().unwrap_or_default();
        if !crate::array_builtin::is_callable(&callback) {
            let text = crate::inspect::inspect(self, &callback, 0)?;
            return Err(self.throw(
                ErrorKind::Type,
                &format!("The \"callback\" argument must be of type function. Received {text}"),
            ));
        }
        // Callback validation precedes delay coercion. Native scheduling uses
        // Node's truncated millisecond delay after its range clamp.
        let delay = if timeout {
            let delay = delay_of(self, args)?;
            Some(if (1.0..=TIMEOUT_MAX).contains(&delay) {
                crate::float::trunc(delay)
            } else {
                1.0
            })
        } else {
            None
        };
        let skipped = if timeout { 2 } else { 1 };
        Ok(Value::Object(self.schedule_callback(
            Callback {
                function: callback,
                args: args.iter().skip(skipped).cloned().collect(),
            },
            delay,
            repeat,
            Rc::new(Cell::new(true)),
        )))
    }

    /// Internal timeouts bypass source-level delay coercion and share a native
    /// handle's reference state. Filesystem polling accepts the full u32 range.
    pub(crate) fn native_timeout(
        &mut self,
        callback: Object,
        delay: f64,
        referenced: Rc<Cell<bool>>,
    ) -> Object {
        self.schedule_callback(
            Callback {
                function: Value::Object(callback),
                args: Vec::new(),
            },
            Some(delay),
            false,
            referenced,
        )
    }

    fn schedule_callback(
        &mut self,
        callback: Callback,
        delay: Option<f64>,
        repeat: bool,
        referenced: Rc<Cell<bool>>,
    ) -> Object {
        self.event_loop.next_id += 1;
        let prototype = if delay.is_some() {
            self.timeout_prototype.clone()
        } else {
            self.immediate_prototype.clone()
        };
        let state = Rc::new(Timer {
            id: Cell::new(self.event_loop.next_id),
            callback: RefCell::new(Some(callback)),
            delay,
            repeat,
            referenced,
            running: Cell::new(false),
            context: self.async_context.clone(),
        });
        let handle = Object::new(Some(prototype), Kind::Timer(state.clone()));
        self.activate(handle.clone(), state);
        handle
    }

    /// Handles can cancel even completed timeouts. Primitive ids only refer
    /// to currently scheduled timeouts; a refreshed completed timer gets a
    /// new id, so an old numeric id cannot cancel its new activation.
    pub(crate) fn clear(&mut self, value: &Value) {
        let state = match value {
            Value::Object(object) => match &object.data().kind {
                Kind::Timer(state) => Some(state.clone()),
                _ => None,
            },
            Value::Number(id) => whole(*id).and_then(|id| {
                self.event_loop
                    .tasks
                    .get(&id)
                    .map(|task| task.state.clone())
            }),
            Value::String(text) => whole(crate::number::parse(text)).and_then(|id| {
                self.event_loop
                    .tasks
                    .get(&id)
                    .map(|task| task.state.clone())
            }),
            _ => None,
        };
        if let Some(state) = state {
            self.remove_task(state.id.get());
            let released = state.callback.borrow_mut().take();
            if state.delay.is_none() {
                state.referenced.set(false);
            }
            // Release callbacks and their captures outside the storage borrow.
            drop(released);
        }
    }
}

/// A whole, non-negative id number.
fn whole(id: f64) -> Option<u64> {
    (id >= 0.0 && crate::float::fract(id) == 0.0).then_some(id as u64)
}

/// The native state of a handle method's receiver.
fn handle_state(realm: &Realm, this: &Value) -> Result<Rc<Timer>, Thrown> {
    let state = this
        .as_object()
        .and_then(|object| match &object.data().kind {
            Kind::Timer(state) => Some(state.clone()),
            _ => None,
        });
    state.ok_or_else(|| realm.throw(ErrorKind::Type, "Illegal invocation"))
}

fn set_referenced(realm: &mut Realm, this: Value, referenced: bool) -> Completion {
    let state = handle_state(realm, &this)?;
    // Completed/cancelled immediates cannot become referenced again.
    if state.delay.is_some() || state.callback.borrow().is_some() {
        state.referenced.set(referenced);
    }
    Ok(this)
}

fn has_ref(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    Ok(Value::Boolean(handle_state(realm, &this)?.referenced.get()))
}

/// Restarts an active or completed timeout, including from its own callback.
/// Explicitly cleared timeouts have no callback and cannot be reactivated.
fn refresh(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let state = handle_state(realm, &this)?;
    if state.delay.is_some() && state.callback.borrow().is_some() {
        let id = state.id.get();
        if !state.running.get() && !realm.event_loop.tasks.contains_key(&id) {
            realm.event_loop.next_id += 1;
            state.id.set(realm.event_loop.next_id);
        }
        realm.activate(this.as_object().expect("a timer handle").clone(), state);
    }
    Ok(this)
}

fn delay_of(realm: &mut Realm, args: &[Value]) -> Result<f64, Thrown> {
    match args.get(1) {
        None | Some(Value::Undefined) => Ok(1.0),
        Some(delay) => ops::to_number(realm, delay),
    }
}

/// A native function body.
type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

pub(crate) fn install(realm: &mut Realm) {
    for prototype in [
        realm.timeout_prototype.clone(),
        realm.immediate_prototype.clone(),
    ] {
        realm.method(&prototype, "ref", 0, |realm, this, _| {
            set_referenced(realm, this, true)
        });
        realm.method(&prototype, "unref", 0, |realm, this, _| {
            set_referenced(realm, this, false)
        });
        realm.method(&prototype, "hasRef", 0, has_ref);
    }
    let timeout = realm.timeout_prototype.clone();
    realm.method(&timeout, "refresh", 0, refresh);
    realm.method(&timeout, "close", 0, |realm, this, _| {
        realm.clear(&this);
        Ok(this)
    });
    let primitive = realm.function("[Symbol.toPrimitive]", 0, false, |realm, this, _| {
        Ok(Value::Number(handle_state(realm, &this)?.id.get() as f64))
    });
    timeout.define(
        Key::Symbol(realm.symbols.to_primitive.clone()),
        Property::hidden(Value::Object(primitive)),
    );
    let globals: [(&str, u32, Native); 7] = [
        ("setTimeout", 2, |realm, _, args| {
            realm.schedule(args, true, false)
        }),
        ("setInterval", 2, |realm, _, args| {
            realm.schedule(args, true, true)
        }),
        ("setImmediate", 1, |realm, _, args| {
            realm.schedule(args, false, false)
        }),
        ("clearTimeout", 1, clear_global),
        ("clearInterval", 1, clear_global),
        ("clearImmediate", 1, clear_global),
        ("queueMicrotask", 1, |realm, _, args| {
            let callback = args.first().cloned().unwrap_or_default();
            if !crate::array_builtin::is_callable(&callback) {
                let text = crate::inspect::inspect(realm, &callback, 0)?;
                return Err(realm.throw(
                    ErrorKind::Type,
                    &format!("The \"callback\" argument must be of type function. Received {text}"),
                ));
            }
            realm.enqueue(Box::new(move |realm| {
                ops::call(realm, &callback, Value::Undefined, &[]).map(drop)
            }));
            Ok(Value::Undefined)
        }),
    ];
    for (name, length, code) in globals {
        let function = realm.function(name, length, false, code);
        realm.set_global(name, Value::Object(function));
    }
}

fn clear_global(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    realm.clear(args.first().unwrap_or(&Value::Undefined));
    Ok(Value::Undefined)
}

#[cfg(test)]
mod tests;
