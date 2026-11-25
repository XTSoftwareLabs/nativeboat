//! Native argument-list calls and bound function operations. No source text
//! is evaluated; targets are existing compiled/native callable objects.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Kind, NativeCode, Object, Property, Slot};
use crate::{Completion, ErrorKind, JsStr, Realm, Thrown, Value, ops};
use alloc::rc::Rc;

// An explicit implementation resource limit, checked before allocation/index
// reads. This is not the language's ToLength bound (2^53 - 1).
const MAX_CALL_ARGUMENTS: usize = 1 << 16;

pub(crate) fn install(realm: &Realm) {
    realm.method(&realm.function_prototype, "apply", 2, apply);
    realm.method(&realm.function_prototype, "bind", 1, bind);
    realm.method(&realm.function_prototype, "toString", 0, to_string);
    let has_instance = realm.function("[Symbol.hasInstance]", 1, false, |realm, this, args| {
        ops::ordinary_has_instance(realm, args.first().unwrap_or(&Value::Undefined), &this)
            .map(Value::Boolean)
    });
    realm.function_prototype.define(
        Key::Symbol(realm.symbols.has_instance.clone()),
        Property {
            slot: Slot::Data {
                value: Value::Object(has_instance),
                writable: false,
            },
            enumerable: false,
            configurable: false,
        },
    );
    for (name, value) in [("name", Value::str("")), ("length", Value::Number(0.0))] {
        realm.function_prototype.define(
            Key::name(name),
            Property {
                slot: Slot::Data {
                    value,
                    writable: false,
                },
                enumerable: false,
                configurable: true,
            },
        );
    }
    install_constructor(realm);
    // Intrinsic catalogues retain `eval` without executing it. Keep its
    // identity and the language's non-string passthrough semantics, while
    // rejecting source text under the same policy as `Function`.
    realm.method(&realm.global, "eval", 1, |realm, _, args| {
        let value = args.first().cloned().unwrap_or_default();
        if matches!(value, Value::String(_)) {
            return Err(realm.throw(
                ErrorKind::Eval,
                "Code generation from strings disallowed for this context",
            ));
        }
        Ok(value)
    });
}

/// `Function`. Without an engine, source text cannot become code, so calls
/// and construction fail as V8 does when code generation from strings is
/// disallowed. Two sources need no code generation and are supported: no
/// parameters and an empty body, and the `Function('return this')` idiom
/// for the global object.
fn install_constructor(realm: &Realm) {
    let constructor = realm.function("Function", 1, true, |realm, _, args| {
        let mut parts = Vec::new();
        for value in args {
            parts.push(ops::to_string(realm, value)?.to_rust_lossy());
        }
        let body = parts.pop().unwrap_or_default();
        if parts.is_empty() && body.trim().is_empty() {
            let empty = realm.function("anonymous", 0, true, |_, _, _| Ok(Value::Undefined));
            return Ok(Value::Object(empty));
        }
        if parts.is_empty() && body.trim().trim_end_matches(';').trim_end() == "return this" {
            let global = realm.function("anonymous", 0, true, |realm, _, _| {
                Ok(Value::Object(realm.global.clone()))
            });
            return Ok(Value::Object(global));
        }
        Err(realm.throw(
            ErrorKind::Eval,
            "Code generation from strings disallowed for this context",
        ))
    });
    constructor.define(
        Key::name("prototype"),
        Property {
            slot: Slot::Data {
                value: Value::Object(realm.function_prototype.clone()),
                writable: false,
            },
            enumerable: false,
            configurable: false,
        },
    );
    realm.function_prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(constructor.clone())),
    );
    realm.set_global("Function", Value::Object(constructor));
}

fn callable(realm: &Realm, value: &Value) -> Result<(Object, bool), Thrown> {
    match value.as_object().and_then(Object::code) {
        Some((_, constructor)) => Ok((
            value.as_object().expect("callable object").clone(),
            constructor,
        )),
        None => Err(realm.throw(ErrorKind::Type, "value is not a function")),
    }
}

/// `Function.prototype.toString`: the source text the compiler kept for a
/// function whose text the program reads, otherwise the form the language
/// gives functions without source text, `function name() { [native code] }`.
fn to_string(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let Value::Object(function) = &this else {
        return Err(not_a_function(realm));
    };
    if function.code().is_none() {
        return Err(not_a_function(realm));
    }
    if let Some(Property {
        slot: Slot::Data { value, .. },
        ..
    }) = function.private_property(&realm.symbols.source_text)
    {
        return Ok(value);
    }
    // Bound functions have no name of their own in this form, as in V8.
    if matches!(function.data().kind, Kind::BoundFunction { .. }) {
        return Ok(Value::str("function () { [native code] }"));
    }
    Ok(Value::str(&crate::weak_builtin::receiver_text(&this)))
}

fn not_a_function(realm: &Realm) -> Thrown {
    realm.throw(
        ErrorKind::Type,
        "Function.prototype.toString requires that 'this' be a Function",
    )
}

/// Gives a compiled function the source text `Function.prototype.toString`
/// returns for it. The compiler keeps the text of the functions whose text
/// a program reads, as programs that run a function's text elsewhere do.
pub fn with_source_text(realm: &Realm, function: Value, text: &'static str) -> Value {
    if let Value::Object(object) = &function {
        let text = JsStr::from(text);
        object.set_private(
            &realm.symbols.source_text,
            Property::hidden(Value::String(text.clone())),
        );
        // Scripts made from the text run the function last made from it.
        let mut functions = realm.source_functions.borrow_mut();
        match functions.iter_mut().find(|(known, _)| *known == text) {
            Some((_, latest)) => *latest = function.clone(),
            None => functions.push((text, function.clone())),
        }
    }
    function
}

/// The functions whose text the program read, as `[text, function]` pairs.
pub(crate) fn source_functions(realm: &mut Realm) -> Value {
    let pairs = realm
        .source_functions
        .borrow()
        .iter()
        .map(|(text, function)| (text.clone(), function.clone()))
        .collect::<Vec<_>>();
    let pairs = pairs
        .into_iter()
        .map(|(text, function)| Value::Object(realm.array(vec![Value::String(text), function])))
        .collect();
    Value::Object(realm.array(pairs))
}

fn check_count(realm: &Realm, count: usize) -> Result<(), Thrown> {
    if count > MAX_CALL_ARGUMENTS {
        Err(realm.throw(ErrorKind::Range, "Native call argument limit exceeded"))
    } else {
        Ok(())
    }
}

pub(crate) fn prepend(
    realm: &Realm,
    leading: &[Value],
    args: &[Value],
) -> Result<Vec<Value>, Thrown> {
    let count = leading.len().saturating_add(args.len());
    check_count(realm, count)?;
    let mut result = Vec::with_capacity(count);
    result.extend_from_slice(leading);
    result.extend_from_slice(args);
    Ok(result)
}

fn apply(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    callable(realm, &this)?;
    let receiver = args.first().cloned().unwrap_or_default();
    let array = args.get(1).cloned().unwrap_or_default();
    if array.is_nullish() {
        return ops::call(realm, &this, receiver, &[]);
    }
    if !matches!(array, Value::Object(_)) {
        return Err(realm.throw(ErrorKind::Type, "apply arguments must be an object"));
    }
    let values = list_from_array_like(realm, &array)?;
    ops::call(realm, &this, receiver, &values)
}

/// `CreateListFromArrayLike` for an object.
pub(crate) fn list_from_array_like(realm: &mut Realm, array: &Value) -> Result<Vec<Value>, Thrown> {
    let length = ops::get_named(realm, array, "length")?;
    let length =
        crate::float::trunc(ops::to_number(realm, &length)?).clamp(0.0, 9_007_199_254_740_991.0);
    // NaN becomes zero as in ToLength. The saturating float conversion also
    // keeps resource checking defined for the ToLength maximum on all targets.
    let count = length as usize;
    check_count(realm, count)?;
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        values.push(ops::get(realm, array, &Key::Index(index as u32))?);
    }
    Ok(values)
}

fn bind(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (target, constructor) = callable(realm, &this)?;
    let receiver = args.first().cloned().unwrap_or_default();
    let leading = args.get(1..).unwrap_or(&[]);
    check_count(realm, leading.len())?;
    let arguments: Rc<[Value]> = Rc::from(leading);
    let call_target = target.clone();
    let call_arguments = arguments.clone();
    let code: NativeCode = Rc::new(move |realm, _, args| {
        let args = prepend(realm, &call_arguments, args)?;
        ops::call(
            realm,
            &Value::Object(call_target.clone()),
            receiver.clone(),
            &args,
        )
    });
    // Snapshot the target's prototype before observable metadata getters.
    let function = Object::new(
        target.data().prototype.clone(),
        Kind::BoundFunction {
            code,
            target: target.clone(),
            arguments: arguments.clone(),
            constructor,
        },
    );
    let mut length = 0.0;
    if target.own_property(&Key::name("length")).is_some()
        && let Value::Number(value) = ops::get_named(realm, &this, "length")?
    {
        length = (crate::float::trunc(value) - arguments.len() as f64).max(0.0);
    }
    let name = match ops::get_named(realm, &this, "name")? {
        Value::String(name) => name,
        _ => JsStr::default(),
    };
    let fixed = |value| Property {
        slot: Slot::Data {
            value,
            writable: false,
        },
        enumerable: false,
        configurable: true,
    };
    function.define(Key::name("length"), fixed(Value::Number(length)));
    function.define(
        Key::name("name"),
        fixed(Value::String(JsStr::from("bound ").concat(&name))),
    );
    Ok(Value::Object(function))
}

#[cfg(test)]
mod tests;
