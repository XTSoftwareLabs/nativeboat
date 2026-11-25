//! The iteration protocol (`for`-`of`, spread, destructuring) and the
//! built-in array and string iterators, plus `for`-`in` key enumeration and
//! the property copying behind object spread and rest.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Kind, Object, Property};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::string::JsStr;
use crate::table::HashSet;
use crate::value::{Completion, Thrown, Value};

/// What a built-in array iterator yields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Yield {
    /// Indexes.
    Keys,
    /// Elements.
    Values,
    /// `[index, element]` pairs.
    Entries,
}

/// The internal state of a built-in iterator object.
#[derive(Debug, Clone)]
pub enum State {
    /// An array iterator over any array-like target.
    Array {
        /// The iterated object, retained for Node 22's live length reads.
        target: Value,
        /// The next index; 2⁵³ − 1 marks permanent exhaustion.
        index: u64,
        /// What it yields.
        yields: Yield,
    },
    /// A string iterator, by code points.
    String {
        /// The iterated text; `None` once exhausted.
        text: Option<JsStr>,
        /// The next code unit.
        index: usize,
    },
}

/// An iterator being consumed: `GetIterator`'s record.
#[derive(Debug)]
pub struct Iteration {
    iterator: Value,
    next: Value,
    done: bool,
    /// Whether `next` is the built-in array (`Some(false)`) or string
    /// (`Some(true)`) iterator `next`, whose result objects nothing can
    /// observe, so steps skip them.
    builtin: Option<bool>,
}

fn type_error(realm: &Realm, message: &str) -> Thrown {
    realm.throw(ErrorKind::Type, message)
}

/// `GetIterator(value, sync)`.
pub fn iterate(realm: &mut Realm, value: &Value) -> Result<Iteration, Thrown> {
    if value.is_nullish() {
        return Err(type_error(realm, "value is not iterable"));
    }
    let key = Key::Symbol(realm.symbols.iterator.clone());
    let method = ops::get(realm, value, &key)?;
    if method.is_nullish() {
        return Err(type_error(realm, "value is not iterable"));
    }
    iterate_method(realm, value, &method)
}

/// `GetIteratorFromMethod`: iterates `value` with an already retrieved
/// `Symbol.iterator` method.
pub fn iterate_method(
    realm: &mut Realm,
    value: &Value,
    method: &Value,
) -> Result<Iteration, Thrown> {
    let iterator = ops::call(realm, method, value.clone(), &[])?;
    if !matches!(iterator, Value::Object(_)) {
        return Err(type_error(
            realm,
            "Result of the Symbol.iterator method is not an object",
        ));
    }
    let next = ops::get_named(realm, &iterator, "next")?;
    let builtin = match &next {
        Value::Object(function) => realm
            .iterator_next
            .iter()
            .position(|builtin| builtin == function)
            .map(|index| index == 1),
        _ => None,
    };
    Ok(Iteration {
        iterator,
        next,
        done: false,
        builtin,
    })
}

impl Iteration {
    /// `IteratorStepValue`: the next value, or `None` when done. An error
    /// from the iterator itself marks it done without closing it.
    pub fn step(&mut self, realm: &mut Realm) -> Result<Option<Value>, Thrown> {
        if let Some(string) = self.builtin {
            if self.done {
                return Ok(None);
            }
            self.done = true;
            let value = advance(realm, &self.iterator, string)?;
            self.done = value.is_none();
            return Ok(value);
        }
        let Some(result) = self.next_result(realm)? else {
            return Ok(None);
        };
        let value = ops::get_named(realm, &result, "value")?;
        self.done = false;
        Ok(Some(value))
    }

    /// An array pattern's elision advances without reading `value`.
    pub fn skip(&mut self, realm: &mut Realm) -> Result<(), Thrown> {
        if self.next_result(realm)?.is_some() {
            self.done = false;
        }
        Ok(())
    }

    fn next_result(&mut self, realm: &mut Realm) -> Result<Option<Value>, Thrown> {
        if self.done {
            return Ok(None);
        }
        self.done = true;
        let result = ops::call(realm, &self.next, self.iterator.clone(), &[])?;
        if !matches!(result, Value::Object(_)) {
            return Err(type_error(realm, "Iterator result is not an object"));
        }
        let done = ops::get_named(realm, &result, "done")?;
        if ops::to_boolean(&done) {
            return Ok(None);
        }
        Ok(Some(result))
    }

    /// `IteratorClose` after a `break` or `return`: calls `return()` and
    /// checks its result.
    pub fn close(&mut self, realm: &mut Realm) -> Result<(), Thrown> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        let method = ops::get_named(realm, &self.iterator, "return")?;
        if method.is_nullish() {
            return Ok(());
        }
        let result = ops::call(realm, &method, self.iterator.clone(), &[])?;
        if !matches!(result, Value::Object(_)) {
            return Err(type_error(realm, "Iterator result is not an object"));
        }
        Ok(())
    }

    /// `IteratorClose` after a throw: calls `return()`, ignoring anything it
    /// throws, and gives back the original thrown value.
    pub fn close_thrown(&mut self, realm: &mut Realm, thrown: Thrown) -> Thrown {
        // A generator's internal signal represents a return completion,
        // which cleanup failure replaces, rather than a source throw.
        if crate::coroutine::is_return(realm, &thrown) {
            return self.close(realm).err().unwrap_or(thrown);
        }
        let _ = self.close(realm);
        thrown
    }
}

/// `GetIterator(value, async)`, preferring `Symbol.asyncIterator` and
/// otherwise adapting a synchronous iterator. The adapter's promise jobs
/// remain observable even for synchronous values and failures.
pub fn iterate_async(realm: &mut Realm, value: &Value) -> Result<AsyncIteration, Thrown> {
    if value.is_nullish() {
        return Err(type_error(realm, "value is not async iterable"));
    }
    let method = ops::get(
        realm,
        value,
        &Key::Symbol(realm.symbols.async_iterator.clone()),
    )?;
    let sync = method.is_nullish();
    let record = if sync {
        iterate(realm, value)?
    } else {
        iterate_method(realm, value, &method)?
    };
    Ok(AsyncIteration { record, sync })
}

/// The iterator record consumed by a compiled `for await` loop.
#[derive(Debug)]
pub struct AsyncIteration {
    record: Iteration,
    sync: bool,
}

/// The iterator/next pair for async `yield*`. A synchronous iterator gets
/// native Async-from-Sync methods, including the ES2024 missing-throw rule
/// used by Node 22: reject the sent reason without closing the iterator.
pub(crate) fn delegation_async(realm: &mut Realm, value: &Value) -> Result<(Value, Value), Thrown> {
    let iteration = iterate_async(realm, value)?;
    if !iteration.sync {
        return Ok((iteration.record.iterator, iteration.record.next));
    }
    let target = iteration.record.iterator;
    let next = iteration.record.next;
    let wrapper = Object::new(Some(realm.async_iterator_prototype.clone()), Kind::Ordinary);
    let iterator = target.clone();
    let next = realm.function("next", 1, false, move |realm, _, args| {
        adapt_sync(realm, |realm| {
            ops::call(realm, &next, iterator.clone(), args)
        })
    });
    wrapper.define(
        Key::name("next"),
        Property::hidden(Value::Object(next.clone())),
    );
    let iterator = target.clone();
    let finish = realm.function("return", 1, false, move |realm, _, args| {
        let method = ops::get_named(realm, &iterator, "return");
        if method.as_ref().is_ok_and(Value::is_nullish) {
            let step = result(realm, args.first().cloned().unwrap_or_default(), true);
            return crate::promise_builtin::resolved(realm, step)
                .map(|(promise, _)| Value::Object(promise));
        }
        adapt_sync(realm, |realm| {
            ops::call(realm, &method?, iterator.clone(), args)
        })
    });
    wrapper.define(Key::name("return"), Property::hidden(Value::Object(finish)));
    let throw = realm.function("throw", 1, false, move |realm, _, args| {
        adapt_sync(realm, |realm| {
            let method = ops::get_named(realm, &target, "throw")?;
            if method.is_nullish() {
                return Err(Thrown(args.first().cloned().unwrap_or_default()));
            }
            ops::call(realm, &method, target.clone(), args)
        })
    });
    wrapper.define(Key::name("throw"), Property::hidden(Value::Object(throw)));
    Ok((Value::Object(wrapper), Value::Object(next)))
}

/// An Async-from-Sync method returns an intrinsic promise, including when
/// calling the synchronous iterator or reading its result throws. Node 22
/// follows ES2024: a rejected yielded value does not close the sync iterator.
fn adapt_sync(realm: &mut Realm, operation: impl FnOnce(&mut Realm) -> Completion) -> Completion {
    let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
    let adapted = (|| {
        let step = operation(realm)?;
        if !matches!(step, Value::Object(_)) {
            return Err(type_error(realm, "Iterator result is not an object"));
        }
        let done = ops::to_boolean(&ops::get_named(realm, &step, "done")?);
        let value = ops::get_named(realm, &step, "value")?;
        let wrapper = crate::promise_builtin::resolved(realm, value)?;
        let fulfilled = realm.function("", 1, false, move |realm, _, args| {
            let value = args.first().cloned().unwrap_or_default();
            let result = result(realm, value, done);
            ops::call(realm, &resolve, Value::Undefined, &[result])
        });
        crate::promise_builtin::react(realm, &wrapper, Value::Object(fulfilled), reject.clone());
        Ok(())
    })();
    if let Err(thrown) = adapted {
        ops::call(realm, &reject, Value::Undefined, &[thrown.0])?;
    }
    Ok(promise)
}

impl AsyncIteration {
    /// Awaits the next result, checking `done` before reading `value`.
    /// Only the synchronous adapter awaits the yielded value itself.
    pub fn step(&mut self, realm: &mut Realm) -> Result<Option<Value>, Thrown> {
        if self.record.done {
            return Ok(None);
        }
        self.record.done = true;
        let next = if self.sync {
            adapt_sync(realm, |realm| {
                ops::call(realm, &self.record.next, self.record.iterator.clone(), &[])
            })?
        } else {
            ops::call(realm, &self.record.next, self.record.iterator.clone(), &[])?
        };
        let result = crate::coroutine::await_value(realm, next)?;
        if !matches!(result, Value::Object(_)) {
            return Err(type_error(realm, "Iterator result is not an object"));
        }
        if ops::to_boolean(&ops::get_named(realm, &result, "done")?) {
            return Ok(None);
        }
        let value = ops::get_named(realm, &result, "value")?;
        self.record.done = false;
        Ok(Some(value))
    }

    /// `AsyncIteratorClose`: an abrupt loop exit awaits `return()`, while
    /// exhaustion and failures obtaining the next value do not close it.
    pub fn close(&mut self, realm: &mut Realm) -> Result<(), Thrown> {
        if self.record.done {
            return Ok(());
        }
        self.record.done = true;
        let iterator = self.record.iterator.clone();
        let returned = if self.sync {
            let method = ops::get_named(realm, &iterator, "return");
            // A missing sync return resolves immediately, without the
            // value-unwrapping reaction used by a present return method.
            match method {
                Ok(method) if method.is_nullish() => result(realm, Value::Undefined, true),
                method => adapt_sync(realm, |realm| ops::call(realm, &method?, iterator, &[]))?,
            }
        } else {
            let method = ops::get_named(realm, &iterator, "return")?;
            if method.is_nullish() {
                return Ok(());
            }
            ops::call(realm, &method, iterator, &[])?
        };
        let result = crate::coroutine::await_value(realm, returned)?;
        if !matches!(result, Value::Object(_)) {
            return Err(type_error(realm, "Iterator result is not an object"));
        }
        Ok(())
    }

    /// Closing must preserve an existing thrown completion even when
    /// `return` throws, rejects or returns a primitive. A generator's
    /// return signal is a return completion, which cleanup can replace.
    pub fn close_thrown(&mut self, realm: &mut Realm, thrown: Thrown) -> Thrown {
        if crate::coroutine::is_return(realm, &thrown) {
            return self.close(realm).err().unwrap_or(thrown);
        }
        let _ = self.close(realm);
        thrown
    }
}

/// Spread: every remaining value of an iterable.
pub fn collect(realm: &mut Realm, value: &Value) -> Result<Vec<Value>, Thrown> {
    let mut iteration = iterate(realm, value)?;
    let mut values = Vec::new();
    while let Some(value) = iteration.step(realm)? {
        values.push(value);
    }
    Ok(values)
}

/// The rest of an iteration, for `[first, ...rest]` patterns.
pub fn rest(realm: &mut Realm, iteration: &mut Iteration) -> Result<Vec<Value>, Thrown> {
    let mut values = Vec::new();
    while let Some(value) = iteration.step(realm)? {
        values.push(value);
    }
    Ok(values)
}

fn iterator_object(realm: &Realm, state: State, string: bool) -> Value {
    let prototype = if string {
        &realm.string_iterator_prototype
    } else {
        &realm.array_iterator_prototype
    };
    Value::Object(Object::new(Some(prototype.clone()), Kind::Iterator(state)))
}

/// A built-in array iterator over a target.
pub fn array_iterator(realm: &mut Realm, target: Value, yields: Yield) -> Completion {
    let target = Value::Object(ops::to_object(realm, &target)?);
    let state = State::Array {
        target,
        index: 0,
        yields,
    };
    Ok(iterator_object(realm, state, false))
}

/// A built-in string iterator.
pub fn string_iterator(realm: &Realm, text: JsStr) -> Value {
    let state = State::String {
        text: Some(text),
        index: 0,
    };
    iterator_object(realm, state, true)
}

/// `{ value, done }`.
fn result(realm: &mut Realm, value: Value, done: bool) -> Value {
    let object = realm.object();
    object.define(realm.name_key("value"), Property::data(value));
    object.define(realm.name_key("done"), Property::data(Value::Boolean(done)));
    Value::Object(object)
}

/// `%ArrayIteratorPrototype%.next` and `%StringIteratorPrototype%.next`.
pub(crate) fn next(realm: &mut Realm, this: Value, string: bool) -> Completion {
    let value = advance(realm, &this, string)?;
    let done = value.is_none();
    Ok(result(realm, value.unwrap_or_default(), done))
}

/// Advances a built-in iterator: its next value, or `None` once done.
fn advance(realm: &mut Realm, this: &Value, string: bool) -> Result<Option<Value>, Thrown> {
    let state = this
        .as_object()
        .and_then(|object| match &object.data().kind {
            Kind::Iterator(state @ State::Array { .. }) if !string => Some(state.clone()),
            Kind::Iterator(state @ State::String { .. }) if string => Some(state.clone()),
            _ => None,
        });
    let Some(state) = state else {
        return Err(type_error(realm, "next called on an incompatible receiver"));
    };
    let (value, next) = match state {
        State::Array {
            target,
            index,
            yields,
        } => {
            let length = crate::array_builtin::length_of(realm, &target)?;
            if index >= length {
                let done = State::Array {
                    target,
                    index: (1 << 53) - 1,
                    yields,
                };
                (None, done)
            } else {
                // Node 22 advances before Get: throwing or reentrant element
                // getters must observe the next index, without an outer call
                // overwriting a nested next()'s progress afterwards.
                this.as_object().expect("checked above").data_mut().kind =
                    Kind::Iterator(State::Array {
                        target: target.clone(),
                        index: index + 1,
                        yields,
                    });
                let key = Value::Number(index as f64);
                let property = crate::array_builtin::index_key(index);
                let value = match yields {
                    Yield::Keys => key,
                    Yield::Values => ops::get(realm, &target, &property)?,
                    Yield::Entries => {
                        let element = ops::get(realm, &target, &property)?;
                        Value::Object(realm.array(vec![key, element]))
                    }
                };
                return Ok(Some(value));
            }
        }
        State::String {
            text: Some(text),
            index,
        } => {
            let units = text.units();
            if index >= units.len() {
                (None, State::String { text: None, index })
            } else {
                let pair = units[index] & 0xFC00 == 0xD800
                    && units
                        .get(index + 1)
                        .is_some_and(|unit| unit & 0xFC00 == 0xDC00);
                let end = index + if pair { 2 } else { 1 };
                let value = Value::String(JsStr::from_units(&units[index..end]));
                let next = State::String {
                    text: Some(text),
                    index: end,
                };
                (Some(value), next)
            }
        }
        exhausted => (None, exhausted),
    };
    let object = this.as_object().expect("checked above");
    object.data_mut().kind = Kind::Iterator(next);
    Ok(value)
}

/// The string keys a `for`-`in` loop visits: enumerable keys of the value
/// and its prototypes, each once, shadowed keys excluded.
pub fn for_in_keys(realm: &mut Realm, value: &Value) -> Result<Vec<Key>, Thrown> {
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    let mut current = match value {
        Value::Undefined | Value::Null => return Ok(keys),
        Value::String(text) => {
            for index in 0..text.len() as u32 {
                seen.insert(Key::Index(index));
                keys.push(Key::Index(index));
            }
            Some(realm.string_prototype.clone())
        }
        Value::Number(_) => Some(realm.number_prototype.clone()),
        Value::Boolean(_) => Some(realm.boolean_prototype.clone()),
        Value::Symbol(_) => Some(realm.symbol_prototype.clone()),
        Value::BigInt(_) => Some(realm.bigint_prototype.clone()),
        Value::Object(object) => Some(object.clone()),
    };
    while let Some(object) = current {
        for key in ops::own_property_keys(realm, &object)? {
            if !matches!(key, Key::Symbol(_)) && seen.insert(key.clone()) {
                let enumerable = ops::get_own_property(realm, &object, &key)?
                    .is_some_and(|property| property.enumerable);
                if enumerable {
                    keys.push(key);
                }
            }
        }
        current = ops::get_prototype_of(realm, &object)?;
    }
    Ok(keys)
}

/// `CopyDataProperties`: object spread and object rest patterns.
pub fn copy_properties(
    realm: &mut Realm,
    target: &Object,
    source: &Value,
    excluded: &[Key],
) -> Result<(), Thrown> {
    let keys: Vec<Key> = match source {
        Value::String(text) => (0..text.len() as u32).map(Key::Index).collect(),
        Value::Object(object) => ops::own_property_keys(realm, object)?,
        _ => Vec::new(),
    };
    for key in keys {
        if excluded.contains(&key) {
            continue;
        }
        let enumerable = match source {
            Value::Object(object) => ops::get_own_property(realm, object, &key)?
                .is_some_and(|property| property.enumerable),
            _ => true,
        };
        if enumerable {
            let value = ops::get(realm, source, &key)?;
            target.define(key, Property::data(value));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
