//! Generators, async functions and async generators. Their compiled bodies
//! are ordinary native
//! code running on a fiber of their own (kernel32's fibers), so `yield` and
//! `await` suspend the whole body, with every construct that compiles
//! elsewhere (loops, `try`/`finally`, destructuring) working unchanged.
//! Generators follow ECMAScript 2024's `next`/`return`/`throw` states and
//! `yield*` delegation; async functions return a promise and resume through
//! promise reactions. Async generators queue `next`/`return`/`throw`
//! requests, each settling its own promise, as ECMAScript 2024's
//! AsyncGenerator objects do. A generator's `return()` unwinds its body through
//! `finally` blocks with an internal signal that `catch` clauses do not
//! catch. Abandoned suspended bodies are discarded without running their
//! `finally` blocks, as in JavaScript; values on their stacks are not freed.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Kind, Object, Property, Slot, WeakObject};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::value::{Completion, Thrown, Value};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::{Rc, Weak};
use core::cell::{Cell, RefCell};
use core::ffi::c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn IsThreadAFiber() -> i32;
    fn ConvertThreadToFiber(parameter: *mut c_void) -> *mut c_void;
    fn CreateFiberEx(
        commit: usize,
        reserve: usize,
        flags: u32,
        start: unsafe extern "system" fn(*mut c_void),
        parameter: *mut c_void,
    ) -> *mut c_void;
    fn SwitchToFiber(fiber: *mut c_void);
    fn DeleteFiber(fiber: *mut c_void);
}

/// Saves floating-point state across switches.
const FIBER_FLAG_FLOAT_SWITCH: u32 = 1;
/// Each body's stack: committed on demand within this reservation, which
/// matches the depth the native call limit allows.
const STACK_RESERVE: usize = 64 << 20;

/// The running fiber, converting the thread on first use.
fn current_fiber() -> *mut c_void {
    // SAFETY: IsThreadAFiber has no preconditions; ConvertThreadToFiber is
    // called only on a thread that is not yet a fiber. Afterwards the TEB's
    // fiber data slot (gs:[0x20] on x64) holds the running fiber.
    unsafe {
        if IsThreadAFiber() == 0 {
            return ConvertThreadToFiber(core::ptr::null_mut());
        }
        let fiber: *mut c_void;
        core::arch::asm!("mov {}, gs:[0x20]", out(reg) fiber, options(nostack, readonly, preserves_flags));
        fiber
    }
}

/// A compiled body, run once on its fiber.
pub type Body = Box<dyn FnOnce(&mut Realm) -> Completion>;

/// What a suspended body receives when it continues.
#[derive(Debug, Clone)]
pub enum Resumption {
    /// `next(value)` or a fulfilled await.
    Next(Value),
    /// `throw(value)` or a rejected await.
    Throw(Value),
    /// `return(value)`.
    Return(Value),
}

/// What the body gives its caller when it suspends or finishes.
#[derive(Debug)]
enum Output {
    /// `yield value`, or `await` (whose reactions resume the body).
    Yield(Value),
    /// A delegated iterator result, passed on unchanged.
    YieldResult(Value),
    /// The body's completion.
    Done(Completion),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Start,
    Suspended,
    Running,
    Done,
}

/// How many finished fibers a realm keeps for later bodies: creating and
/// deleting a fiber, with its reserved stack, costs more than many await
/// steps, and an editor's keystroke runs dozens of async functions.
const IDLE_FIBERS: usize = 16;

/// The coroutine a fiber runs: a fiber runs one body after another.
pub(crate) struct FiberSlot {
    coroutine: Cell<*const Coroutine>,
}

/// A realm's fibers whose bodies finished, with their slots.
#[derive(Default)]
pub(crate) struct FiberPool(Vec<(*mut c_void, *mut FiberSlot)>);

impl core::fmt::Debug for FiberPool {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "FiberPool({})", self.0.len())
    }
}

impl Drop for FiberPool {
    fn drop(&mut self) {
        for (fiber, slot) in self.0.drain(..) {
            // SAFETY: idle fibers are not running and their slots were
            // allocated with them.
            unsafe {
                DeleteFiber(fiber);
                drop(Box::from_raw(slot));
            }
        }
    }
}

/// A body with its fiber and the values exchanged at each switch.
pub struct Coroutine {
    fiber: Cell<*mut c_void>,
    /// The fiber's slot, which names this coroutine to it.
    slot: Cell<*mut FiberSlot>,
    caller: Cell<*mut c_void>,
    realm: Cell<*mut Realm>,
    status: Cell<Status>,
    body: RefCell<Option<Body>>,
    resumption: RefCell<Option<Resumption>>,
    output: RefCell<Option<Output>>,
    /// The value a `return()` resumption returns with.
    returning: RefCell<Option<Value>>,
    /// What kind of body this is, which decides how an await resumes it.
    mode: Mode,
    /// Whether the body suspended at an `await` rather than a `yield`.
    awaiting: Cell<bool>,
}

/// The kind of body a coroutine runs.
enum Mode {
    Generator,
    /// An async function's promise capability: resolve and reject.
    Async(Value, Value),
    /// An async generator, resumed by its request queue.
    AsyncGenerator(RefCell<Weak<AsyncGenerator>>),
}

impl core::fmt::Debug for Coroutine {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "Coroutine({:?})", self.status.get())
    }
}

impl Drop for Coroutine {
    fn drop(&mut self) {
        let fiber = self.fiber.get();
        if !fiber.is_null() {
            // SAFETY: the fiber is not running: a running body holds a
            // reference to its coroutine on the realm's stack. Its slot was
            // allocated with it.
            unsafe {
                DeleteFiber(fiber);
                drop(Box::from_raw(self.slot.get()));
            }
        }
    }
}

impl Coroutine {
    fn new(body: Body, mode: Mode) -> Rc<Coroutine> {
        Rc::new(Coroutine {
            fiber: Cell::new(core::ptr::null_mut()),
            slot: Cell::new(core::ptr::null_mut()),
            caller: Cell::new(core::ptr::null_mut()),
            realm: Cell::new(core::ptr::null_mut()),
            status: Cell::new(Status::Start),
            body: RefCell::new(Some(body)),
            resumption: RefCell::new(None),
            output: RefCell::new(None),
            returning: RefCell::new(None),
            mode,
            awaiting: Cell::new(false),
        })
    }
}

/// The fiber entry: runs its slot's body and reports its completion, then
/// waits in the realm's pool for the next body.
unsafe extern "system" fn start(parameter: *mut c_void) {
    let slot = parameter as *mut FiberSlot;
    loop {
        // SAFETY: the slot names the coroutine, kept alive by the realm's
        // stack of running coroutines while its fiber runs.
        let coroutine = unsafe { &*(*slot).coroutine.get() };
        let body = coroutine
            .body
            .borrow_mut()
            .take()
            .expect("a body starts once");
        // SAFETY: the resuming caller stored its realm, which outlives the run.
        let realm = unsafe { &mut *coroutine.realm.get() };
        coroutine.resumption.borrow_mut().take();
        let completion = body(realm);
        coroutine.status.set(Status::Done);
        *coroutine.output.borrow_mut() = Some(Output::Done(completion));
        let caller = coroutine.caller.get();
        // SAFETY: as above.
        let realm = unsafe { &mut *coroutine.realm.get() };
        if realm.idle_fibers.0.len() < IDLE_FIBERS {
            // The fiber leaves the finished coroutine for the pool, where a
            // later body takes it up and switches here again.
            realm.idle_fibers.0.push((coroutine.fiber.get(), slot));
            coroutine.fiber.set(core::ptr::null_mut());
            coroutine.slot.set(core::ptr::null_mut());
            // SAFETY: the caller fiber is suspended in `resume`.
            unsafe { SwitchToFiber(caller) };
        } else {
            // A finished body is never resumed; its coroutine deletes the
            // fiber.
            loop {
                // SAFETY: as above.
                unsafe { SwitchToFiber(caller) };
            }
        }
    }
}

/// Runs the body until it suspends or finishes.
fn resume(realm: &mut Realm, coroutine: &Rc<Coroutine>, resumption: Resumption) -> Output {
    if coroutine.status.get() == Status::Done {
        return Output::Done(Ok(Value::Undefined));
    }
    if coroutine.fiber.get().is_null() {
        let (fiber, slot) = match realm.idle_fibers.0.pop() {
            Some((fiber, slot)) => {
                // SAFETY: an idle fiber's slot is live and its fiber waits.
                unsafe { (*slot).coroutine.set(Rc::as_ptr(coroutine)) };
                (fiber, slot)
            }
            None => {
                let slot = Box::into_raw(Box::new(FiberSlot {
                    coroutine: Cell::new(Rc::as_ptr(coroutine)),
                }));
                // SAFETY: the start routine and slot stay valid while the
                // fiber exists; whoever holds the fiber deletes both.
                let fiber = unsafe {
                    CreateFiberEx(
                        0,
                        STACK_RESERVE,
                        FIBER_FLAG_FLOAT_SWITCH,
                        start,
                        slot.cast(),
                    )
                };
                assert!(!fiber.is_null(), "out of memory creating a fiber");
                (fiber, slot)
            }
        };
        coroutine.fiber.set(fiber);
        coroutine.slot.set(slot);
    }
    coroutine.caller.set(current_fiber());
    coroutine.realm.set(realm as *mut Realm);
    coroutine.status.set(Status::Running);
    *coroutine.resumption.borrow_mut() = Some(resumption);
    realm.coroutines.push(coroutine.clone());
    // SAFETY: the fiber was created above and is not running.
    unsafe { SwitchToFiber(coroutine.fiber.get()) };
    realm.coroutines.pop();
    coroutine
        .output
        .borrow_mut()
        .take()
        .expect("a switched-out body leaves an output")
}

/// Suspends the running body with an output until it is resumed.
fn suspend(realm: &mut Realm, output: Output) -> Result<Resumption, Thrown> {
    let Some(coroutine) = realm.coroutines.last().cloned() else {
        return Err(realm.throw(
            ErrorKind::Syntax,
            "yield and await are only valid in generators and async functions",
        ));
    };
    coroutine.status.set(Status::Suspended);
    *coroutine.output.borrow_mut() = Some(output);
    // SAFETY: the caller fiber is suspended in `resume`.
    unsafe { SwitchToFiber(coroutine.caller.get()) };
    Ok(coroutine
        .resumption
        .borrow_mut()
        .take()
        .expect("a resumed body receives a resumption"))
}

/// The completion a resumption produces inside the body.
fn receive(realm: &mut Realm, resumption: Resumption) -> Completion {
    match resumption {
        Resumption::Next(value) => Ok(value),
        Resumption::Throw(value) => Err(Thrown(value)),
        Resumption::Return(value) => {
            let coroutine = realm.coroutines.last().cloned().expect("a body is running");
            *coroutine.returning.borrow_mut() = Some(value);
            Err(Thrown(Value::Object(realm.return_signal.clone())))
        }
    }
}

/// `yield value`: the value `next` resumes with.
pub fn yield_value(realm: &mut Realm, value: Value) -> Completion {
    let resumption = suspend(realm, Output::Yield(value))?;
    receive(realm, resumption)
}

/// Whether a thrown value is a generator's `return()` unwinding its body,
/// which `catch` clauses pass on.
pub fn is_return(realm: &Realm, thrown: &Thrown) -> bool {
    matches!(&thrown.0, Value::Object(object) if *object == realm.return_signal)
}

fn result_object(realm: &Realm, value: Value, done: bool) -> Value {
    let result = realm.object();
    result.define(crate::string::static_key!("value"), Property::data(value));
    result.define(
        crate::string::static_key!("done"),
        Property::data(Value::Boolean(done)),
    );
    Value::Object(result)
}

/// `yield* iterable`: delegates `next`, `throw` and `return` to the
/// iterable's iterator, passing its results on unchanged; the iterator's
/// final value is the expression's value.
pub fn yield_delegate(realm: &mut Realm, iterable: Value) -> Completion {
    let key = Key::Symbol(realm.symbols.iterator.clone());
    let method = ops::get(realm, &iterable, &key)?;
    if !crate::array_builtin::is_callable(&method) {
        return Err(realm.throw(ErrorKind::Type, "yield* operand is not iterable"));
    }
    let iterator = ops::call(realm, &method, iterable, &[])?;
    let next = ops::get_named(realm, &iterator, "next")?;
    let mut received = Resumption::Next(Value::Undefined);
    loop {
        let (result, returning) = match received {
            Resumption::Next(value) => {
                (ops::call(realm, &next, iterator.clone(), &[value])?, false)
            }
            Resumption::Throw(value) => {
                let throw = ops::get_named(realm, &iterator, "throw")?;
                if throw.is_nullish() {
                    close(realm, &iterator)?;
                    return Err(realm.throw(
                        ErrorKind::Type,
                        "The iterator does not provide a 'throw' method",
                    ));
                }
                (ops::call(realm, &throw, iterator.clone(), &[value])?, false)
            }
            Resumption::Return(value) => {
                let finish = ops::get_named(realm, &iterator, "return")?;
                if finish.is_nullish() {
                    return receive(realm, Resumption::Return(value));
                }
                (ops::call(realm, &finish, iterator.clone(), &[value])?, true)
            }
        };
        if !matches!(result, Value::Object(_)) {
            return Err(realm.throw(ErrorKind::Type, "Iterator result is not an object"));
        }
        let done = ops::get_named(realm, &result, "done")?;
        if ops::to_boolean(&done) {
            let value = ops::get_named(realm, &result, "value")?;
            if returning {
                return receive(realm, Resumption::Return(value));
            }
            return Ok(value);
        }
        received = suspend(realm, Output::YieldResult(result))?;
    }
}

/// Async `yield*` for Node 22: awaits iterator results but passes an async
/// iterator's values through unchanged. The sync adapter unwraps its values.
pub fn async_yield_delegate(realm: &mut Realm, iterable: Value) -> Completion {
    let (iterator, next) = crate::iter::delegation_async(realm, &iterable)?;
    let mut received = Resumption::Next(Value::Undefined);
    loop {
        let (result, returning) = match received {
            Resumption::Next(value) => {
                (ops::call(realm, &next, iterator.clone(), &[value])?, false)
            }
            Resumption::Throw(value) => {
                let throw = ops::get_named(realm, &iterator, "throw")?;
                if throw.is_nullish() {
                    close_async_delegate(realm, &iterator)?;
                    return Err(realm.throw(
                        ErrorKind::Type,
                        "The iterator does not provide a 'throw' method",
                    ));
                }
                (ops::call(realm, &throw, iterator.clone(), &[value])?, false)
            }
            Resumption::Return(value) => {
                let finish = ops::get_named(realm, &iterator, "return")?;
                if finish.is_nullish() {
                    let value = await_value(realm, value)?;
                    return receive(realm, Resumption::Return(value));
                }
                (ops::call(realm, &finish, iterator.clone(), &[value])?, true)
            }
        };
        let result = await_value(realm, result)?;
        if !matches!(result, Value::Object(_)) {
            return Err(realm.throw(ErrorKind::Type, "Iterator result is not an object"));
        }
        let done = ops::to_boolean(&ops::get_named(realm, &result, "done")?);
        let value = ops::get_named(realm, &result, "value")?;
        if done {
            return if returning {
                receive(realm, Resumption::Return(value))
            } else {
                Ok(value)
            };
        }
        received = match suspend(realm, Output::Yield(value))? {
            Resumption::Return(value) => match await_value(realm, value) {
                Ok(value) => Resumption::Return(value),
                // Node 22 resumes delegated next with a rejected return's
                // reason. Ordinary async yield still resumes with a throw.
                Err(thrown) => Resumption::Next(thrown.0),
            },
            resumption => resumption,
        };
    }
}

fn close_async_delegate(realm: &mut Realm, iterator: &Value) -> Result<(), Thrown> {
    let finish = ops::get_named(realm, iterator, "return")?;
    if finish.is_nullish() {
        return Ok(());
    }
    let result = ops::call(realm, &finish, iterator.clone(), &[])?;
    let result = await_value(realm, result)?;
    if !matches!(result, Value::Object(_)) {
        return Err(realm.throw(ErrorKind::Type, "Iterator result is not an object"));
    }
    Ok(())
}

/// `IteratorClose` with a normal completion.
fn close(realm: &mut Realm, iterator: &Value) -> Result<(), Thrown> {
    let finish = ops::get_named(realm, iterator, "return")?;
    if finish.is_nullish() {
        return Ok(());
    }
    let result = ops::call(realm, &finish, iterator.clone(), &[])?;
    if !matches!(result, Value::Object(_)) {
        return Err(realm.throw(ErrorKind::Type, "Iterator result is not an object"));
    }
    Ok(())
}

/// The coroutine of a generator object.
fn generator_of(realm: &Realm, this: &Value, method: &str) -> Result<Rc<Coroutine>, Thrown> {
    match this.as_object().map(|object| match &object.data().kind {
        Kind::Generator(coroutine) => Some(coroutine.clone()),
        _ => None,
    }) {
        Some(Some(coroutine)) => Ok(coroutine),
        _ => Err(realm.throw(
            ErrorKind::Type,
            &format!("{method} method called on incompatible receiver"),
        )),
    }
}

/// `GeneratorResume` and `GeneratorResumeAbrupt`.
fn generator_step(
    realm: &mut Realm,
    this: &Value,
    resumption: Resumption,
    method: &str,
) -> Completion {
    let coroutine = generator_of(realm, this, method)?;
    match coroutine.status.get() {
        Status::Running => {
            return Err(realm.throw(ErrorKind::Type, "Generator is already running"));
        }
        Status::Start if !matches!(resumption, Resumption::Next(_)) => {
            coroutine.status.set(Status::Done);
            coroutine.body.borrow_mut().take();
        }
        Status::Start | Status::Suspended => {
            return match resume(realm, &coroutine, resumption) {
                Output::Yield(value) => Ok(result_object(realm, value, false)),
                Output::YieldResult(result) => Ok(result),
                Output::Done(Ok(value)) => Ok(result_object(realm, value, true)),
                Output::Done(Err(thrown)) if is_return(realm, &thrown) => {
                    let value = coroutine.returning.borrow_mut().take().unwrap_or_default();
                    Ok(result_object(realm, value, true))
                }
                Output::Done(Err(thrown)) => Err(thrown),
            };
        }
        Status::Done => {}
    }
    match resumption {
        Resumption::Next(_) => Ok(result_object(realm, Value::Undefined, true)),
        Resumption::Return(value) => Ok(result_object(realm, value, true)),
        Resumption::Throw(value) => Err(Thrown(value)),
    }
}

/// A body's parameters bound at call time, returning the body itself.
pub type Start = dyn Fn(&mut Realm, Value, &[Value]) -> Result<Body, Thrown>;

/// A generator function: calling it binds its parameters and returns a
/// generator object whose prototype is the function's `prototype`.
pub fn generator_function(
    realm: &Realm,
    name: &str,
    length: u32,
    start: impl Fn(&mut Realm, Value, &[Value]) -> Result<Body, Thrown> + 'static,
) -> Object {
    make_generator_function(realm, name, length, Box::new(start))
}

fn make_generator_function(realm: &Realm, name: &str, length: u32, start: Box<Start>) -> Object {
    let own: Rc<RefCell<Option<WeakObject>>> = Rc::default();
    let reference = own.clone();
    let function = realm.function(name, length, false, move |realm, this, args| {
        let body = start(realm, this, args)?;
        let function = reference.borrow().as_ref().and_then(WeakObject::upgrade);
        let function = function.expect("a called function is alive");
        let prototype = ops::get_named(realm, &Value::Object(function), "prototype")?;
        let prototype = match prototype {
            Value::Object(prototype) => prototype,
            _ => realm.generator_prototype.clone(),
        };
        let generator = Object::new(
            Some(prototype),
            Kind::Generator(Coroutine::new(body, Mode::Generator)),
        );
        Ok(Value::Object(generator))
    });
    *own.borrow_mut() = Some(function.downgrade());
    let prototype = Object::new(Some(realm.generator_prototype.clone()), Kind::Ordinary);
    function.define(
        Key::name("prototype"),
        Property {
            slot: Slot::Data {
                value: Value::Object(prototype),
                writable: true,
            },
            enumerable: false,
            configurable: false,
        },
    );
    function
}

/// An async function: calling it runs its body until the first `await`
/// and returns a promise of its completion. Errors binding parameters
/// reject the promise.
pub fn async_function(
    realm: &Realm,
    name: &str,
    length: u32,
    start: impl Fn(&mut Realm, Value, &[Value]) -> Result<Body, Thrown> + 'static,
) -> Object {
    make_async_function(realm, name, length, Box::new(start))
}

fn make_async_function(realm: &Realm, name: &str, length: u32, start: Box<Start>) -> Object {
    realm.function(name, length, false, move |realm, this, args| {
        let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
        match start(realm, this, args) {
            Ok(body) => {
                let settle = (resolve.clone(), reject.clone());
                let coroutine = Coroutine::new(body, Mode::Async(resolve, reject));
                let first = Resumption::Next(Value::Undefined);
                async_step(realm, &coroutine, &settle, first)?;
            }
            Err(thrown) => {
                ops::call(realm, &reject, Value::Undefined, &[thrown.0])?;
            }
        }
        Ok(promise)
    })
}

/// Runs an async body until it awaits or finishes, settling its promise.
fn async_step(
    realm: &mut Realm,
    coroutine: &Rc<Coroutine>,
    (resolve, reject): &(Value, Value),
    resumption: Resumption,
) -> Result<(), Thrown> {
    match resume(realm, coroutine, resumption) {
        Output::Done(Ok(value)) => ops::call(realm, resolve, Value::Undefined, &[value]).map(drop),
        Output::Done(Err(thrown)) => {
            ops::call(realm, reject, Value::Undefined, &[thrown.0]).map(drop)
        }
        // Suspended at an await, whose reactions resume it.
        _ => Ok(()),
    }
}

/// What an awaited promise's reactions resume: an async function's body,
/// or an async generator, which the reactions keep alive.
#[derive(Clone)]
enum Awaiter {
    Function(Rc<Coroutine>, (Value, Value)),
    Generator(Rc<AsyncGenerator>),
}

/// Continues a body after its awaited promise settles.
fn after_await(realm: &mut Realm, awaiter: &Awaiter, resumption: Resumption) -> Result<(), Thrown> {
    match awaiter {
        Awaiter::Generator(generator) => drive(realm, generator, resumption),
        Awaiter::Function(coroutine, settle) => async_step(realm, coroutine, settle, resumption),
    }
}

/// `await value`: suspends the async body until the value settles.
pub fn await_value(realm: &mut Realm, value: Value) -> Completion {
    let coroutine = realm.coroutines.last().cloned();
    let awaiter = match coroutine
        .as_ref()
        .map(|coroutine| (coroutine, &coroutine.mode))
    {
        Some((coroutine, Mode::Async(resolve, reject))) => {
            Awaiter::Function(coroutine.clone(), (resolve.clone(), reject.clone()))
        }
        // A generator body runs inside `drive`, which holds its generator.
        Some((_, Mode::AsyncGenerator(generator))) => {
            Awaiter::Generator(generator.borrow().upgrade().expect("a driven generator"))
        }
        _ => return Err(realm.throw(ErrorKind::Syntax, "await is only valid in async functions")),
    };
    let coroutine = coroutine.expect("an awaiting body");
    let promise = crate::promise_builtin::resolved(realm, value)?;
    let (fulfilled, rejected) = (awaiter.clone(), awaiter);
    let on_fulfilled = realm.function("", 1, false, move |realm, _, args| {
        let value = args.first().cloned().unwrap_or_default();
        after_await(realm, &fulfilled, Resumption::Next(value))?;
        Ok(Value::Undefined)
    });
    let on_rejected = realm.function("", 1, false, move |realm, _, args| {
        let reason = args.first().cloned().unwrap_or_default();
        after_await(realm, &rejected, Resumption::Throw(reason))?;
        Ok(Value::Undefined)
    });
    crate::promise_builtin::react(
        realm,
        &promise,
        Value::Object(on_fulfilled),
        Value::Object(on_rejected),
    );
    coroutine.awaiting.set(true);
    let resumption = suspend(realm, Output::Yield(Value::Undefined))?;
    receive(realm, resumption)
}

/// An async generator's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    SuspendedStart,
    SuspendedYield,
    Executing,
    AwaitingReturn,
    Completed,
}

/// A queued `next`, `return` or `throw` with its promise's settle functions.
struct Request {
    resumption: Resumption,
    resolve: Value,
    reject: Value,
}

/// An async generator object's body, state and request queue.
pub struct AsyncGenerator {
    coroutine: Rc<Coroutine>,
    state: Cell<State>,
    queue: RefCell<VecDeque<Request>>,
}

impl core::fmt::Debug for AsyncGenerator {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "AsyncGenerator({:?})", self.state.get())
    }
}

/// `yield value` in an async generator: the value is awaited, settles the
/// current request, and the body waits for the next one. A `return`
/// request's value is awaited before the body returns.
pub fn async_yield(realm: &mut Realm, value: Value) -> Completion {
    let resumption = async_yield_resumption(realm, value)?;
    receive(realm, resumption)
}

fn async_yield_resumption(realm: &mut Realm, value: Value) -> Result<Resumption, Thrown> {
    let value = match await_value(realm, value) {
        Ok(value) => value,
        Err(thrown) => return Ok(Resumption::Throw(thrown.0)),
    };
    let resumption = match suspend(realm, Output::Yield(value))? {
        Resumption::Return(value) => match await_value(realm, value) {
            Ok(value) => Resumption::Return(value),
            Err(thrown) => Resumption::Throw(thrown.0),
        },
        resumption => resumption,
    };
    Ok(resumption)
}

/// Settles the oldest request.
fn settle_front(
    realm: &mut Realm,
    generator: &AsyncGenerator,
    result: Result<Value, Value>,
) -> Result<(), Thrown> {
    let request = generator
        .queue
        .borrow_mut()
        .pop_front()
        .expect("a request is pending");
    let (function, value) = match result {
        Ok(value) => (request.resolve, value),
        Err(reason) => (request.reject, reason),
    };
    ops::call(realm, &function, Value::Undefined, &[value]).map(drop)
}

/// Runs the body with a request's resumption until it awaits, yields or
/// finishes, then serves the remaining requests.
fn drive(
    realm: &mut Realm,
    generator: &Rc<AsyncGenerator>,
    resumption: Resumption,
) -> Result<(), Thrown> {
    generator.state.set(State::Executing);
    let output = resume(realm, &generator.coroutine, resumption);
    if generator.coroutine.awaiting.take() {
        return Ok(());
    }
    let result = match output {
        Output::Yield(value) => {
            generator.state.set(State::SuspendedYield);
            Ok(result_object(realm, value, false))
        }
        Output::YieldResult(result) => {
            generator.state.set(State::SuspendedYield);
            Ok(result)
        }
        Output::Done(completion) => {
            generator.state.set(State::Completed);
            match completion {
                Ok(value) => Ok(result_object(realm, value, true)),
                Err(thrown) if is_return(realm, &thrown) => {
                    let value = generator
                        .coroutine
                        .returning
                        .borrow_mut()
                        .take()
                        .unwrap_or_default();
                    Ok(result_object(realm, value, true))
                }
                Err(thrown) => Err(thrown.0),
            }
        }
    };
    settle_front(realm, generator, result)?;
    resume_next(realm, generator)
}

/// `AsyncGeneratorResumeNext`: serves queued requests while the body is
/// not running.
fn resume_next(realm: &mut Realm, generator: &Rc<AsyncGenerator>) -> Result<(), Thrown> {
    loop {
        let state = generator.state.get();
        if matches!(state, State::Executing | State::AwaitingReturn) {
            return Ok(());
        }
        let Some(resumption) = generator
            .queue
            .borrow()
            .front()
            .map(|request| request.resumption.clone())
        else {
            return Ok(());
        };
        match (state, resumption) {
            (State::SuspendedStart, Resumption::Next(value))
            | (State::SuspendedYield, Resumption::Next(value)) => {
                return drive(realm, generator, Resumption::Next(value));
            }
            (State::SuspendedYield, resumption) => return drive(realm, generator, resumption),
            // A request before the body starts, other than `next`,
            // completes the generator without running it.
            (State::SuspendedStart, _) => generator.state.set(State::Completed),
            (_, Resumption::Next(_)) => {
                let result = result_object(realm, Value::Undefined, true);
                settle_front(realm, generator, Ok(result))?;
            }
            (_, Resumption::Throw(reason)) => settle_front(realm, generator, Err(reason))?,
            (_, Resumption::Return(value)) => {
                // The returned value is awaited first.
                generator.state.set(State::AwaitingReturn);
                let promise = match crate::promise_builtin::resolved(realm, value) {
                    Ok(promise) => promise,
                    Err(thrown) => {
                        generator.state.set(State::Completed);
                        settle_front(realm, generator, Err(thrown.0))?;
                        continue;
                    }
                };
                let (fulfilled, rejected) = (generator.clone(), generator.clone());
                let on_fulfilled = realm.function("", 1, false, move |realm, _, args| {
                    fulfilled.state.set(State::Completed);
                    let result =
                        result_object(realm, args.first().cloned().unwrap_or_default(), true);
                    settle_front(realm, &fulfilled, Ok(result))?;
                    resume_next(realm, &fulfilled)?;
                    Ok(Value::Undefined)
                });
                let on_rejected = realm.function("", 1, false, move |realm, _, args| {
                    rejected.state.set(State::Completed);
                    let reason = args.first().cloned().unwrap_or_default();
                    settle_front(realm, &rejected, Err(reason))?;
                    resume_next(realm, &rejected)?;
                    Ok(Value::Undefined)
                });
                crate::promise_builtin::react(
                    realm,
                    &promise,
                    Value::Object(on_fulfilled),
                    Value::Object(on_rejected),
                );
                return Ok(());
            }
        }
    }
}

/// `AsyncGeneratorEnqueue`: `next`, `return` and `throw` return a promise
/// of their result; a wrong receiver rejects it.
fn async_generator_request(
    realm: &mut Realm,
    this: &Value,
    resumption: Resumption,
    method: &str,
) -> Completion {
    let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
    let generator = match this.as_object().map(|object| match &object.data().kind {
        Kind::AsyncGenerator(generator) => Some(generator.clone()),
        _ => None,
    }) {
        Some(Some(generator)) => generator,
        _ => {
            let error = realm.error(
                ErrorKind::Type,
                &format!("{method} method called on incompatible receiver"),
            );
            ops::call(realm, &reject, Value::Undefined, &[Value::Object(error)])?;
            return Ok(promise);
        }
    };
    generator.queue.borrow_mut().push_back(Request {
        resumption,
        resolve,
        reject,
    });
    resume_next(realm, &generator)?;
    Ok(promise)
}

/// An async generator function: calling it binds its parameters and
/// returns an async generator object.
pub fn async_generator_function(
    realm: &Realm,
    name: &str,
    length: u32,
    start: impl Fn(&mut Realm, Value, &[Value]) -> Result<Body, Thrown> + 'static,
) -> Object {
    make_async_generator_function(realm, name, length, Box::new(start))
}

fn make_async_generator_function(
    realm: &Realm,
    name: &str,
    length: u32,
    start: Box<Start>,
) -> Object {
    let own: Rc<RefCell<Option<WeakObject>>> = Rc::default();
    let reference = own.clone();
    let function = realm.function(name, length, false, move |realm, this, args| {
        let body = start(realm, this, args)?;
        let function = reference.borrow().as_ref().and_then(WeakObject::upgrade);
        let function = function.expect("a called function is alive");
        let prototype = match ops::get_named(realm, &Value::Object(function), "prototype")? {
            Value::Object(prototype) => prototype,
            _ => realm.async_generator_prototype.clone(),
        };
        let coroutine = Coroutine::new(body, Mode::AsyncGenerator(RefCell::new(Weak::new())));
        let generator = Rc::new(AsyncGenerator {
            coroutine,
            state: Cell::new(State::SuspendedStart),
            queue: RefCell::new(VecDeque::new()),
        });
        if let Mode::AsyncGenerator(owner) = &generator.coroutine.mode {
            *owner.borrow_mut() = Rc::downgrade(&generator);
        }
        Ok(Value::Object(Object::new(
            Some(prototype),
            Kind::AsyncGenerator(generator),
        )))
    });
    *own.borrow_mut() = Some(function.downgrade());
    function.data_mut().prototype = Some(realm.async_generator_function_prototype.clone());
    let prototype = Object::new(
        Some(realm.async_generator_prototype.clone()),
        Kind::Ordinary,
    );
    function.define(
        Key::name("prototype"),
        Property {
            slot: Slot::Data {
                value: Value::Object(prototype),
                writable: true,
            },
            enumerable: false,
            configurable: false,
        },
    );
    function
}

fn tag(realm: &Realm, object: &Object, name: &str) {
    object.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str(name),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

/// `%AsyncIteratorPrototype%`, `%AsyncGeneratorPrototype%` and
/// `%AsyncGeneratorFunction.prototype%`.
fn install_async(realm: &mut Realm) {
    let iterator = realm.async_iterator_prototype.clone();
    let method = realm.function("[Symbol.asyncIterator]", 0, false, |_, this, _| Ok(this));
    iterator.define(
        Key::Symbol(realm.symbols.async_iterator.clone()),
        Property::hidden(Value::Object(method)),
    );
    let prototype = realm.async_generator_prototype.clone();
    prototype.data_mut().prototype = Some(iterator);
    realm.method(&prototype, "next", 1, |realm, this, args| {
        let value = args.first().cloned().unwrap_or_default();
        async_generator_request(realm, &this, Resumption::Next(value), "next")
    });
    realm.method(&prototype, "return", 1, |realm, this, args| {
        let value = args.first().cloned().unwrap_or_default();
        async_generator_request(realm, &this, Resumption::Return(value), "return")
    });
    realm.method(&prototype, "throw", 1, |realm, this, args| {
        let value = args.first().cloned().unwrap_or_default();
        async_generator_request(realm, &this, Resumption::Throw(value), "throw")
    });
    tag(realm, &prototype, "AsyncGenerator");
    let function = realm.async_generator_function_prototype.clone();
    function.data_mut().prototype = Some(realm.function_prototype.clone());
    let fixed = |value: Value| Property {
        slot: Slot::Data {
            value,
            writable: false,
        },
        enumerable: false,
        configurable: true,
    };
    function.define(
        Key::name("prototype"),
        fixed(Value::Object(prototype.clone())),
    );
    prototype.define(
        Key::name("constructor"),
        fixed(Value::Object(function.clone())),
    );
    tag(realm, &function, "AsyncGeneratorFunction");
}

pub(crate) fn install(realm: &mut Realm) {
    install_async(realm);
    let prototype = realm.generator_prototype.clone();
    realm.method(&prototype, "next", 1, |realm, this, args| {
        let value = args.first().cloned().unwrap_or_default();
        generator_step(realm, &this, Resumption::Next(value), "next")
    });
    realm.method(&prototype, "return", 1, |realm, this, args| {
        let value = args.first().cloned().unwrap_or_default();
        generator_step(realm, &this, Resumption::Return(value), "return")
    });
    realm.method(&prototype, "throw", 1, |realm, this, args| {
        let value = args.first().cloned().unwrap_or_default();
        generator_step(realm, &this, Resumption::Throw(value), "throw")
    });
    prototype.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str("Generator"),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

#[cfg(test)]
mod tests;
