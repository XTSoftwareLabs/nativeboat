//! Promises as ECMAScript 2024 specifies them: the constructor and its
//! resolving functions, thenable jobs, `then`/`catch`/`finally` with species
//! constructors, and the `resolve`/`reject`/`all`/`allSettled`/`any`/`race`/
//! `withResolvers` statics. Reactions run as jobs on the realm's queue
//! (see `event_loop`), which also tracks unhandled rejections.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::array_builtin::{is_callable, is_constructor};
use crate::iter;
use crate::object::{Key, Kind, Object, Property};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::value::{Completion, Thrown, Value};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

/// A promise's state and pending reactions.
#[derive(Debug, Default)]
pub struct PromiseData {
    /// The settled result, if any.
    pub state: State,
    fulfill: Vec<Reaction>,
    reject: Vec<Reaction>,
    /// Whether a handler was ever attached (`[[PromiseIsHandled]]`).
    handled: bool,
}

/// `[[PromiseState]]` with `[[PromiseResult]]`.
#[derive(Debug, Default, Clone)]
pub enum State {
    /// Not yet settled.
    #[default]
    Pending,
    /// Fulfilled with a value.
    Fulfilled(Value),
    /// Rejected with a reason.
    Rejected(Value),
}

/// The shared state of a promise object.
pub type Shared = Rc<RefCell<PromiseData>>;

/// `PromiseCapability`.
#[derive(Debug, Clone)]
struct Capability {
    promise: Value,
    resolve: Value,
    reject: Value,
}

/// `PromiseReaction`.
#[derive(Debug, Clone)]
struct Reaction {
    /// Absent for `await`, whose handlers settle nothing.
    capability: Option<Capability>,
    rejection: bool,
    handler: Option<Value>,
    /// The async context frame of the `then` or `await` that registered it.
    context: Value,
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

/// The promise state of a value, if it is a promise (`IsPromise`).
fn promise_of(value: &Value) -> Option<Shared> {
    match &value.as_object()?.data().kind {
        Kind::Promise(shared) => Some(shared.clone()),
        _ => None,
    }
}

fn type_error(realm: &Realm, message: &str) -> Thrown {
    realm.throw(ErrorKind::Type, message)
}

/// A new pending promise with a prototype.
fn allocate(prototype: Object) -> (Object, Shared) {
    let state = Shared::default();
    let promise = Object::new(Some(prototype), Kind::Promise(state.clone()));
    (promise, state)
}

/// `CreateResolvingFunctions`. The functions share the promise until one of
/// them runs, so settling releases it.
fn resolving_functions(realm: &Realm, promise: &Object, state: &Shared) -> (Value, Value) {
    let target = Rc::new(RefCell::new(Some((promise.clone(), state.clone()))));
    let resolving = target.clone();
    let resolve = realm.function("", 1, false, move |realm, _, args| {
        let promise = resolving.borrow_mut().take();
        if let Some((promise, state)) = promise {
            resolve_promise(realm, &promise, &state, arg(args, 0));
        }
        Ok(Value::Undefined)
    });
    let reject = realm.function("", 1, false, move |realm, _, args| {
        let promise = target.borrow_mut().take();
        if let Some((promise, state)) = promise {
            reject_promise(realm, &promise, &state, arg(args, 0));
        }
        Ok(Value::Undefined)
    });
    (Value::Object(resolve), Value::Object(reject))
}

/// The resolve function's steps once it is the first to run.
fn resolve_promise(realm: &mut Realm, promise: &Object, state: &Shared, resolution: Value) {
    if matches!(&resolution, Value::Object(object) if object == promise) {
        let error = type_error(realm, "Chaining cycle detected for promise #<Promise>");
        return reject_promise(realm, promise, state, error.0);
    }
    if !matches!(resolution, Value::Object(_)) {
        return fulfill_promise(realm, state, resolution);
    }
    let then = match ops::get_named(realm, &resolution, "then") {
        Ok(then) => then,
        Err(thrown) => return reject_promise(realm, promise, state, thrown.0),
    };
    if !is_callable(&then) {
        return fulfill_promise(realm, state, resolution);
    }
    // `NewPromiseResolveThenableJob`.
    let (promise, state) = (promise.clone(), state.clone());
    realm.enqueue(Box::new(move |realm| {
        let (resolve, reject) = resolving_functions(realm, &promise, &state);
        if let Err(thrown) = ops::call(realm, &then, resolution, &[resolve, reject.clone()]) {
            ops::call(realm, &reject, Value::Undefined, &[thrown.0])?;
        }
        Ok(())
    }));
}

/// `FulfillPromise`.
fn fulfill_promise(realm: &mut Realm, state: &Shared, value: Value) {
    let reactions = {
        let mut data = state.borrow_mut();
        data.state = State::Fulfilled(value.clone());
        data.reject.clear();
        core::mem::take(&mut data.fulfill)
    };
    trigger(realm, reactions, value);
}

/// `RejectPromise`, tracking the rejection while no handler is attached.
fn reject_promise(realm: &mut Realm, promise: &Object, state: &Shared, reason: Value) {
    let (reactions, handled) = {
        let mut data = state.borrow_mut();
        data.state = State::Rejected(reason.clone());
        data.fulfill.clear();
        (core::mem::take(&mut data.reject), data.handled)
    };
    if !handled {
        realm
            .event_loop
            .unhandled
            .push((promise.clone(), reason.clone()));
    }
    trigger(realm, reactions, reason);
}

/// `TriggerPromiseReactions`.
fn trigger(realm: &mut Realm, reactions: Vec<Reaction>, argument: Value) {
    for reaction in reactions {
        enqueue_reaction(realm, reaction, argument.clone());
    }
}

/// `NewPromiseReactionJob`.
fn enqueue_reaction(realm: &mut Realm, reaction: Reaction, argument: Value) {
    let context = reaction.context.clone();
    realm.enqueue_in(
        context,
        Box::new(move |realm| {
            let result = match &reaction.handler {
                None if reaction.rejection => Err(Thrown(argument)),
                None => Ok(argument),
                Some(handler) => ops::call(realm, handler, Value::Undefined, &[argument]),
            };
            let Some(capability) = reaction.capability else {
                return result.map(drop);
            };
            match result {
                Ok(value) => ops::call(realm, &capability.resolve, Value::Undefined, &[value]),
                Err(thrown) => ops::call(realm, &capability.reject, Value::Undefined, &[thrown.0]),
            }
            .map(drop)
        }),
    );
}

/// `PerformPromiseThen`.
fn perform_then(
    realm: &mut Realm,
    promise: &Object,
    state: &Shared,
    on_fulfilled: &Value,
    on_rejected: &Value,
    capability: Option<Capability>,
) {
    let handler = |value: &Value| is_callable(value).then(|| value.clone());
    let fulfill = Reaction {
        capability: capability.clone(),
        rejection: false,
        handler: handler(on_fulfilled),
        context: realm.async_context.clone(),
    };
    let reject = Reaction {
        capability,
        rejection: true,
        handler: handler(on_rejected),
        context: realm.async_context.clone(),
    };
    let current = {
        let mut data = state.borrow_mut();
        let current = data.state.clone();
        let was_handled = core::mem::replace(&mut data.handled, true);
        if matches!(current, State::Pending) {
            data.fulfill.push(fulfill.clone());
            data.reject.push(reject.clone());
        }
        (current, was_handled)
    };
    match current {
        (State::Pending, _) => {}
        (State::Fulfilled(value), _) => enqueue_reaction(realm, fulfill, value),
        (State::Rejected(reason), was_handled) => {
            if !was_handled {
                realm
                    .event_loop
                    .unhandled
                    .retain(|(rejected, _)| rejected != promise);
            }
            enqueue_reaction(realm, reject, reason);
        }
    }
}

/// `NewPromiseCapability`.
fn new_capability(realm: &mut Realm, constructor: &Value) -> Result<Capability, Thrown> {
    if !is_constructor(constructor) {
        return Err(type_error(
            realm,
            "Promise capability constructor is not a constructor",
        ));
    }
    if constructor.as_object() == realm.promise_constructor.as_ref() {
        let (promise, state) = allocate(realm.promise_prototype.clone());
        let (resolve, reject) = resolving_functions(realm, &promise, &state);
        return Ok(Capability {
            promise: Value::Object(promise),
            resolve,
            reject,
        });
    }
    let slots: Rc<RefCell<(Value, Value)>> = Rc::default();
    let captured = slots.clone();
    let executor = realm.function("", 2, false, move |realm, _, args| {
        let mut slots = captured.borrow_mut();
        if !matches!(slots.0, Value::Undefined) || !matches!(slots.1, Value::Undefined) {
            return Err(type_error(
                realm,
                "Promise executor has already been invoked with non-undefined arguments",
            ));
        }
        *slots = (arg(args, 0), arg(args, 1));
        Ok(Value::Undefined)
    });
    let promise = ops::construct(realm, constructor, &[Value::Object(executor)])?;
    let (resolve, reject) = slots.borrow().clone();
    if !is_callable(&resolve) || !is_callable(&reject) {
        return Err(type_error(
            realm,
            "Promise resolve or reject function is not callable",
        ));
    }
    Ok(Capability {
        promise,
        resolve,
        reject,
    })
}

/// A pending promise of the intrinsic constructor with its resolving
/// functions, as async functions create one.
pub(crate) fn capability(realm: &Realm) -> (Value, Value, Value) {
    let (promise, state) = allocate(realm.promise_prototype.clone());
    let (resolve, reject) = resolving_functions(realm, &promise, &state);
    (Value::Object(promise), resolve, reject)
}

/// `PromiseResolve(%Promise%, value)`, as `await` uses it: always a
/// promise object with its state.
pub(crate) fn resolved(realm: &mut Realm, value: Value) -> Result<(Object, Shared), Thrown> {
    let constructor = Value::Object(realm_promise(realm));
    let promise = promise_resolve(realm, &constructor, value)?;
    let state = promise_of(&promise).expect("PromiseResolve(%Promise%) returns promises");
    let object = promise.as_object().expect("promises are objects").clone();
    Ok((object, state))
}

/// `PerformPromiseThen` without a result promise, for `await`.
pub(crate) fn react(
    realm: &mut Realm,
    (promise, state): &(Object, Shared),
    on_fulfilled: Value,
    on_rejected: Value,
) {
    perform_then(realm, promise, state, &on_fulfilled, &on_rejected, None);
}

/// `SpeciesConstructor(promise, %Promise%)`.
fn species(realm: &mut Realm, object: &Value) -> Completion {
    let constructor = ops::get_named(realm, object, "constructor")?;
    if matches!(constructor, Value::Undefined) {
        return Ok(Value::Object(realm_promise(realm)));
    }
    if !matches!(constructor, Value::Object(_)) {
        return Err(type_error(realm, "object.constructor is not an object"));
    }
    let key = Key::Symbol(realm.symbols.species.clone());
    let species = ops::get(realm, &constructor, &key)?;
    if species.is_nullish() {
        return Ok(Value::Object(realm_promise(realm)));
    }
    if is_constructor(&species) {
        return Ok(species);
    }
    Err(type_error(
        realm,
        "object.constructor[Symbol.species] is not a constructor",
    ))
}

fn realm_promise(realm: &Realm) -> Object {
    realm
        .promise_constructor
        .clone()
        .expect("Promise is installed")
}

/// `PromiseResolve(C, x)`.
fn promise_resolve(realm: &mut Realm, constructor: &Value, value: Value) -> Completion {
    if promise_of(&value).is_some() {
        let own = ops::get_named(realm, &value, "constructor")?;
        if crate::object::same_value(&own, constructor) {
            return Ok(value);
        }
    }
    let capability = new_capability(realm, constructor)?;
    ops::call(realm, &capability.resolve, Value::Undefined, &[value])?;
    Ok(capability.promise)
}

fn this_promise(realm: &Realm, this: &Value, method: &str) -> Result<(Object, Shared), Thrown> {
    match (this.as_object(), promise_of(this)) {
        (Some(promise), Some(state)) => Ok((promise.clone(), state)),
        _ => Err(type_error(
            realm,
            &format!("Method Promise.prototype.{method} called on incompatible receiver"),
        )),
    }
}

fn then(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (promise, state) = this_promise(realm, &this, "then")?;
    let constructor = species(realm, &this)?;
    let capability = new_capability(realm, &constructor)?;
    let result = capability.promise.clone();
    perform_then(
        realm,
        &promise,
        &state,
        &arg(args, 0),
        &arg(args, 1),
        Some(capability),
    );
    Ok(result)
}

/// `Invoke(value, "then", args)`.
fn invoke_then(realm: &mut Realm, value: &Value, args: &[Value]) -> Completion {
    let then = ops::get_named(realm, value, "then")?;
    ops::call(realm, &then, value.clone(), args)
}

fn finally(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    if !matches!(this, Value::Object(_)) {
        return Err(type_error(
            realm,
            "Method Promise.prototype.finally called on incompatible receiver",
        ));
    }
    let constructor = species(realm, &this)?;
    let on_finally = arg(args, 0);
    if !is_callable(&on_finally) {
        return invoke_then(realm, &this, &[on_finally.clone(), on_finally]);
    }
    let (fulfilled_callback, fulfilled_constructor) = (on_finally.clone(), constructor.clone());
    let then_finally = realm.function("", 1, false, move |realm, _, args| {
        let value = arg(args, 0);
        let result = ops::call(realm, &fulfilled_callback, Value::Undefined, &[])?;
        let promise = promise_resolve(realm, &fulfilled_constructor, result)?;
        let thunk = realm.function("", 0, false, move |_, _, _| Ok(value.clone()));
        invoke_then(realm, &promise, &[Value::Object(thunk)])
    });
    let catch_finally = realm.function("", 1, false, move |realm, _, args| {
        let reason = arg(args, 0);
        let result = ops::call(realm, &on_finally, Value::Undefined, &[])?;
        let promise = promise_resolve(realm, &constructor, result)?;
        let thrower = realm.function("", 0, false, move |_, _, _| Err(Thrown(reason.clone())));
        invoke_then(realm, &promise, &[Value::Object(thrower)])
    });
    invoke_then(
        realm,
        &this,
        &[Value::Object(then_finally), Value::Object(catch_finally)],
    )
}

/// Which combinator `PerformPromiseAll` and its relatives run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Combinator {
    All,
    AllSettled,
    Any,
    Race,
}

/// The shared state of `all`, `allSettled` and `any`.
#[derive(Clone)]
struct Collector {
    results: Rc<RefCell<Vec<Value>>>,
    remaining: Rc<Cell<usize>>,
    capability: Capability,
    combinator: Combinator,
}

impl Collector {
    /// An element callback: records its result once and settles the
    /// capability when the last one arrives.
    fn element(
        &self,
        realm: &Realm,
        index: usize,
        rejection: bool,
        called: &Rc<Cell<bool>>,
    ) -> Value {
        let (collector, called) = (self.clone(), called.clone());
        let function = realm.function("", 1, false, move |realm, _, args| {
            if called.replace(true) {
                return Ok(Value::Undefined);
            }
            let value = arg(args, 0);
            let entry = if collector.combinator == Combinator::AllSettled {
                let entry = realm.object();
                let (status, key) = if rejection {
                    ("rejected", "reason")
                } else {
                    ("fulfilled", "value")
                };
                entry.define(Key::name("status"), Property::data(Value::str(status)));
                entry.define(Key::name(key), Property::data(value));
                Value::Object(entry)
            } else {
                value
            };
            collector.results.borrow_mut()[index] = entry;
            collector.finish_one(realm)?;
            Ok(Value::Undefined)
        });
        Value::Object(function)
    }

    /// Counts one more result, settling the capability after the last one.
    fn finish_one(&self, realm: &mut Realm) -> Result<(), Thrown> {
        self.remaining.set(self.remaining.get() - 1);
        if self.remaining.get() > 0 {
            return Ok(());
        }
        let values = self.results.borrow().clone();
        let array = Value::Object(realm.array(values));
        if self.combinator == Combinator::Any {
            let error = realm.error(ErrorKind::Error, "All promises were rejected");
            error.set_prototype(Some(realm.aggregate_error_prototype.clone()));
            error.define(Key::name("errors"), Property::hidden(array));
            let reject = self.capability.reject.clone();
            ops::call(realm, &reject, Value::Undefined, &[Value::Object(error)])?;
        } else {
            let resolve = self.capability.resolve.clone();
            ops::call(realm, &resolve, Value::Undefined, &[array])?;
        }
        Ok(())
    }
}

/// `Promise.all`, `allSettled`, `any` and `race`.
fn combine(realm: &mut Realm, this: Value, args: &[Value], combinator: Combinator) -> Completion {
    let capability = new_capability(realm, &this)?;
    let reject_with = |realm: &mut Realm, thrown: Thrown| -> Completion {
        ops::call(realm, &capability.reject, Value::Undefined, &[thrown.0])?;
        Ok(capability.promise.clone())
    };
    let resolver = match ops::get_named(realm, &this, "resolve") {
        Ok(resolver) if is_callable(&resolver) => resolver,
        Ok(_) => {
            let thrown = type_error(realm, "Promise resolve or reject function is not callable");
            return reject_with(realm, thrown);
        }
        Err(thrown) => return reject_with(realm, thrown),
    };
    let mut iteration = match iter::iterate(realm, &arg(args, 0)) {
        Ok(iteration) => iteration,
        Err(thrown) => return reject_with(realm, thrown),
    };
    let collector = Collector {
        results: Rc::default(),
        remaining: Rc::new(Cell::new(1)),
        capability: capability.clone(),
        combinator,
    };
    let mut index = 0;
    loop {
        let next = match iteration.step(realm) {
            Ok(Some(next)) => next,
            Ok(None) => break,
            Err(thrown) => return reject_with(realm, thrown),
        };
        let added = (|| -> Result<(), Thrown> {
            let promise = ops::call(realm, &resolver, this.clone(), &[next])?;
            if combinator == Combinator::Race {
                invoke_then(
                    realm,
                    &promise,
                    &[capability.resolve.clone(), capability.reject.clone()],
                )?;
                return Ok(());
            }
            collector.results.borrow_mut().push(Value::Undefined);
            let called = Rc::new(Cell::new(false));
            let callback =
                |realm: &Realm, rejection| collector.element(realm, index, rejection, &called);
            let (fulfilled, rejected) = match combinator {
                Combinator::All => (callback(realm, false), capability.reject.clone()),
                Combinator::AllSettled => (callback(realm, false), callback(realm, true)),
                _ => (capability.resolve.clone(), callback(realm, true)),
            };
            collector.remaining.set(collector.remaining.get() + 1);
            invoke_then(realm, &promise, &[fulfilled, rejected])?;
            Ok(())
        })();
        if let Err(thrown) = added {
            let thrown = iteration.close_thrown(realm, thrown);
            return reject_with(realm, thrown);
        }
        index += 1;
    }
    if combinator != Combinator::Race
        && let Err(thrown) = collector.finish_one(realm)
    {
        return reject_with(realm, thrown);
    }
    Ok(capability.promise.clone())
}

/// A native function body.
type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

pub(crate) fn install(realm: &mut Realm) {
    let prototype = realm.promise_prototype.clone();
    let fallback = prototype.clone();
    let constructor = realm.builtin_constructor(
        "Promise",
        1,
        &prototype,
        |realm, _, _| {
            Err(type_error(
                realm,
                "Promise constructor cannot be invoked without 'new'",
            ))
        },
        move |realm, target, args| {
            let executor = arg(args, 0);
            if !is_callable(&executor) {
                let text = crate::inspect::inspect(realm, &executor, 0)?;
                return Err(type_error(
                    realm,
                    &format!("Promise resolver {text} is not a function"),
                ));
            }
            let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
                Value::Object(prototype) => prototype,
                _ => fallback.clone(),
            };
            let (promise, state) = allocate(prototype);
            let (resolve, reject) = resolving_functions(realm, &promise, &state);
            if let Err(thrown) = ops::call(
                realm,
                &executor,
                Value::Undefined,
                &[resolve, reject.clone()],
            ) {
                ops::call(realm, &reject, Value::Undefined, &[thrown.0])?;
            }
            Ok(promise)
        },
    );
    realm.promise_constructor = Some(constructor.clone());
    let species_getter = realm.function("get [Symbol.species]", 0, false, |_, this, _| Ok(this));
    crate::ops::define_accessor(
        &constructor,
        Key::Symbol(realm.symbols.species.clone()),
        Some(species_getter),
        None,
    );
    realm.method(&prototype, "then", 2, then);
    realm.method(&prototype, "catch", 1, |realm, this, args| {
        invoke_then(realm, &this, &[Value::Undefined, arg(args, 0)])
    });
    realm.method(&prototype, "finally", 1, finally);
    prototype.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: crate::object::Slot::Data {
                value: Value::str("Promise"),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let statics: [(&str, u32, Native); 7] = [
        ("all", 1, |realm, this, args| {
            combine(realm, this, args, Combinator::All)
        }),
        ("allSettled", 1, |realm, this, args| {
            combine(realm, this, args, Combinator::AllSettled)
        }),
        ("any", 1, |realm, this, args| {
            combine(realm, this, args, Combinator::Any)
        }),
        ("race", 1, |realm, this, args| {
            combine(realm, this, args, Combinator::Race)
        }),
        ("reject", 1, |realm, this, args| {
            let capability = new_capability(realm, &this)?;
            ops::call(realm, &capability.reject, Value::Undefined, &[arg(args, 0)])?;
            Ok(capability.promise)
        }),
        ("resolve", 1, |realm, this, args| {
            if !matches!(this, Value::Object(_)) {
                return Err(type_error(realm, "PromiseResolve called on non-object"));
            }
            promise_resolve(realm, &this, arg(args, 0))
        }),
        ("withResolvers", 0, |realm, this, _| {
            let capability = new_capability(realm, &this)?;
            let result = realm.object();
            for (name, value) in [
                ("promise", capability.promise),
                ("resolve", capability.resolve),
                ("reject", capability.reject),
            ] {
                result.define(Key::name(name), Property::data(value));
            }
            Ok(Value::Object(result))
        }),
    ];
    for (name, length, code) in statics {
        realm.method(&constructor, name, length, code);
    }
    realm.set_global("Promise", Value::Object(constructor));
}

#[cfg(test)]
mod tests;
