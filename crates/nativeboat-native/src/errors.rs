//! Native standard Error constructors. Calls and construction both create errors.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Slot, Thrown, Value, iter, ops};

#[derive(Clone, Copy)]
enum Form {
    Simple,
    Aggregate,
    Suppressed,
}

pub(crate) fn install(realm: &mut Realm) {
    let base = constructor(realm, "Error", 1, &realm.error_prototype, Form::Simple);
    realm.method(&base, "isError", 1, |_, _, args| {
        Ok(Value::Boolean(
            args.first()
                .and_then(Value::as_object)
                .is_some_and(|object| matches!(object.data().kind, Kind::Error)),
        ))
    });
    install_stack(realm, &base);
    realm.error_constructor = Some(base.clone());
    realm.set_global("Error", Value::Object(base.clone()));
    for (name, length, prototype, form) in [
        ("EvalError", 1, &realm.eval_error_prototype, Form::Simple),
        ("RangeError", 1, &realm.range_error_prototype, Form::Simple),
        (
            "ReferenceError",
            1,
            &realm.reference_error_prototype,
            Form::Simple,
        ),
        (
            "SyntaxError",
            1,
            &realm.syntax_error_prototype,
            Form::Simple,
        ),
        ("TypeError", 1, &realm.type_error_prototype, Form::Simple),
        ("URIError", 1, &realm.uri_error_prototype, Form::Simple),
        (
            "AggregateError",
            2,
            &realm.aggregate_error_prototype,
            Form::Aggregate,
        ),
        (
            "SuppressedError",
            3,
            &realm.suppressed_error_prototype,
            Form::Suppressed,
        ),
    ] {
        let function = constructor(realm, name, length, prototype, form);
        function.data_mut().prototype = Some(base.clone());
        realm.set_global(name, Value::Object(function));
    }
}

fn constructor(realm: &Realm, name: &str, length: u32, prototype: &Object, form: Form) -> Object {
    let call_prototype = prototype.clone();
    let fallback = prototype.clone();
    realm.builtin_constructor(
        name,
        length,
        prototype,
        move |realm, _, args| create(realm, call_prototype.clone(), form, args).map(Value::Object),
        move |realm, new_target, args| {
            let prototype = match ops::get_named(realm, &Value::Object(new_target), "prototype")? {
                Value::Object(prototype) => prototype,
                _ => fallback.clone(),
            };
            create(realm, prototype, form, args)
        },
    )
}

/// V8's stack trace interface: errors get a lazy `stack` accessor, and
/// `Error.captureStackTrace` gives one to any object. Compiled code has no
/// source positions, so a stack holds only its header line, `name: message`
/// as the error reads when the stack is first read.
fn install_stack(realm: &mut Realm, base: &Object) {
    let get = realm.function("get stack", 0, false, |realm, this, _| {
        let Value::Object(object) = &this else {
            return Ok(Value::Undefined);
        };
        let header = crate::builtins::error_to_string(realm, this.clone(), &[])?;
        object.define(Key::name("stack"), Property::hidden(header.clone()));
        Ok(header)
    });
    let set = realm.function("set stack", 1, false, |_, this, args| {
        if let Value::Object(object) = &this {
            object.define(Key::name("stack"), Property::hidden(arg(args, 0).clone()));
        }
        Ok(Value::Undefined)
    });
    realm.error_stack = Some((get, set));
    realm.method(base, "captureStackTrace", 2, |realm, _, args| {
        let Some(target) = args.first().and_then(Value::as_object) else {
            return Err(realm.throw(ErrorKind::Type, "Invalid argument"));
        };
        capture_stack(realm, target);
        Ok(Value::Undefined)
    });
    base.define(
        Key::name("stackTraceLimit"),
        Property::data(Value::Number(10.0)),
    );
}

/// Gives an object V8's lazy `stack` accessor.
pub(crate) fn capture_stack(realm: &Realm, object: &Object) {
    if let Some((get, set)) = &realm.error_stack {
        object.define(
            Key::name("stack"),
            Property {
                slot: Slot::Accessor {
                    get: Some(get.clone()),
                    set: Some(set.clone()),
                },
                enumerable: false,
                configurable: true,
            },
        );
    }
}

fn arg(args: &[Value], index: usize) -> &Value {
    args.get(index).unwrap_or(&Value::Undefined)
}

fn create(
    realm: &mut Realm,
    prototype: Object,
    form: Form,
    args: &[Value],
) -> Result<Object, Thrown> {
    let error = Object::new(Some(prototype), Kind::Error);
    capture_stack(realm, &error);
    let message_index = match form {
        Form::Simple => 0,
        Form::Aggregate => 1,
        Form::Suppressed => 2,
    };
    let message = arg(args, message_index);
    if !matches!(message, Value::Undefined) {
        error.define(
            Key::name("message"),
            Property::hidden(Value::String(ops::to_string(realm, message)?)),
        );
    }
    if !matches!(form, Form::Suppressed) {
        let options = arg(args, message_index + 1);
        if matches!(options, Value::Object(_))
            && ops::has_property(realm, &Key::name("cause"), options)?
        {
            let cause = ops::get_named(realm, options, "cause")?;
            error.define(Key::name("cause"), Property::hidden(cause));
        }
    }
    match form {
        Form::Simple => {}
        Form::Aggregate => {
            let mut iterator = iter::iterate(realm, arg(args, 0))?;
            let mut errors = Vec::new();
            while let Some(value) = iterator.step(realm)? {
                errors.push(value);
            }
            error.define(
                Key::name("errors"),
                Property::hidden(Value::Object(realm.array(errors))),
            );
        }
        Form::Suppressed => {
            error.define(Key::name("error"), Property::hidden(arg(args, 0).clone()));
            error.define(
                Key::name("suppressed"),
                Property::hidden(arg(args, 1).clone()),
            );
        }
    }
    Ok(error)
}

#[cfg(test)]
mod tests;
