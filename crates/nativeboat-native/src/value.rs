//! Dynamic values. Generated code uses these only where analysis has not
//! proven a narrower native type; proven scalars stay plain `f64`/`bool`.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::Object;
use crate::string::JsStr;
use crate::symbol::Symbol;

/// A value of the source language.
#[derive(Clone, Debug, Default)]
pub enum Value {
    /// `undefined`.
    #[default]
    Undefined,
    /// `null`.
    Null,
    /// A boolean.
    Boolean(bool),
    /// An IEEE-754 double, including NaN and both zeros.
    Number(f64),
    /// A UTF-16 string.
    String(JsStr),
    /// A symbol.
    Symbol(Symbol),
    /// An arbitrary-precision integer.
    BigInt(alloc::rc::Rc<crate::bigint::BigInt>),
    /// An object, function or array.
    Object(Object),
}

impl Value {
    /// A string value from Rust text.
    pub fn str(text: &str) -> Value {
        Value::String(JsStr::from(text))
    }

    /// The object, if this is one.
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Object(object) => Some(object),
            _ => None,
        }
    }

    /// Whether this is `undefined` or `null`.
    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Value {
        Value::Number(value)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Value {
        Value::Boolean(value)
    }
}

impl From<JsStr> for Value {
    fn from(value: JsStr) -> Value {
        Value::String(value)
    }
}

impl From<Symbol> for Value {
    fn from(value: Symbol) -> Value {
        Value::Symbol(value)
    }
}

impl From<Object> for Value {
    fn from(value: Object) -> Value {
        Value::Object(value)
    }
}

/// A thrown value travelling up the native call stack.
#[derive(Clone, Debug)]
pub struct Thrown(pub Value);

/// The result of evaluating source behavior: a value, or a thrown value.
pub type Completion = Result<Value, Thrown>;

#[cfg(test)]
mod tests;
