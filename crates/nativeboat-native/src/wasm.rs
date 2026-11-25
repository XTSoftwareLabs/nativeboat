//! WebAssembly modules, run by the runtime itself: [`decode`] reads and
//! validates the binary format, [`compile`] turns each function body into
//! instructions with resolved branches, and [`exec`] instantiates modules
//! and runs their code. The JavaScript API over it is
//! `internalBinding('wasm')` ([`crate::wasm_binding`]) and
//! `internal/webassembly`.
//!
//! The core specification's instructions are supported with the finished
//! proposals JavaScript engines enable: sign extension, saturating float to
//! integer conversion, bulk memory, reference types, multiple values,
//! mutable globals and extended constant expressions. SIMD, threads,
//! exception handling, multiple memories and memory64 are not.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) mod compile;
pub(crate) mod decode;
pub(crate) mod exec;

#[cfg(test)]
mod tests;

/// A value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValType {
    I32,
    I64,
    F32,
    F64,
    FuncRef,
    ExternRef,
}

impl ValType {
    pub(crate) fn from_byte(byte: u8) -> Option<ValType> {
        Some(match byte {
            0x7F => ValType::I32,
            0x7E => ValType::I64,
            0x7D => ValType::F32,
            0x7C => ValType::F64,
            0x70 => ValType::FuncRef,
            0x6F => ValType::ExternRef,
            _ => return None,
        })
    }

    /// The type as the JavaScript API's descriptors name it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            ValType::I32 => "i32",
            ValType::I64 => "i64",
            ValType::F32 => "f32",
            ValType::F64 => "f64",
            ValType::FuncRef => "anyfunc",
            ValType::ExternRef => "externref",
        }
    }

    /// A type the JavaScript API names: `anyfunc` and `funcref` are the
    /// same type.
    pub(crate) fn parse(name: &str) -> Option<ValType> {
        Some(match name {
            "i32" => ValType::I32,
            "i64" => ValType::I64,
            "f32" => ValType::F32,
            "f64" => ValType::F64,
            "anyfunc" | "funcref" => ValType::FuncRef,
            "externref" => ValType::ExternRef,
            _ => return None,
        })
    }

    pub(crate) fn is_reference(self) -> bool {
        matches!(self, ValType::FuncRef | ValType::ExternRef)
    }
}

/// A function's parameter and result types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FuncType {
    pub(crate) params: Vec<ValType>,
    pub(crate) results: Vec<ValType>,
}

/// A memory's or table's size limits, in pages or elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    pub(crate) min: u32,
    pub(crate) max: Option<u32>,
}

impl Limits {
    /// Whether limits satisfy an import's: at least its minimum, and within
    /// its maximum when it has one.
    pub(crate) fn matches(&self, import: &Limits) -> bool {
        self.min >= import.min
            && match import.max {
                None => true,
                Some(max) => self.max.is_some_and(|own| own <= max),
            }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableType {
    pub(crate) element: ValType,
    pub(crate) limits: Limits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GlobalType {
    pub(crate) ty: ValType,
    pub(crate) mutable: bool,
}
