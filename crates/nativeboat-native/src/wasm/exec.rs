//! WebAssembly's runtime structure and execution: functions, tables,
//! memories and globals, instantiation, and an interpreter for compiled
//! bodies. Imported JavaScript functions are called through the realm, and
//! exceptions they throw pass through WebAssembly frames unchanged.

#[allow(unused_imports)]
use crate::prelude::*;

use super::compile::{Body, Op, Target};
use super::decode::{ConstExpr, ConstOp, ImportDesc, Module, SegmentMode};
use super::{FuncType, GlobalType, Limits, TableType, ValType};
use crate::object::Object;
use crate::realm::Realm;
use crate::typed_array_builtin::Bytes;
use crate::value::{Thrown, Value};
use alloc::rc::Rc;
use core::cell::{OnceCell, RefCell};

/// The size of a memory page.
pub(crate) const PAGE: usize = 65536;
/// The most pages a 32-bit memory can have.
pub(crate) const MAX_PAGES: u32 = 65536;
/// The most frames a call may nest WebAssembly calls to.
const MAX_FRAMES: usize = 60_000;
/// The most operand and local slots execution may use.
const MAX_SLOTS: usize = 16 << 20;

/// A reference.
#[derive(Clone, Debug)]
pub(crate) enum Ref {
    Func(Rc<Func>),
    Extern(Value),
}

/// A value, as it crosses into or out of WebAssembly.
#[derive(Clone, Debug)]
pub(crate) enum Val {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    Ref(Option<Ref>),
}

impl Val {
    /// The default value of a type.
    pub(crate) fn default_of(ty: ValType) -> Val {
        match ty {
            ValType::I32 => Val::I32(0),
            ValType::I64 => Val::I64(0),
            ValType::F32 => Val::F32(0),
            ValType::F64 => Val::F64(0),
            ValType::FuncRef | ValType::ExternRef => Val::Ref(None),
        }
    }
}

/// A function: one a module defines, or a host (JavaScript) function.
#[derive(Debug)]
pub(crate) enum Func {
    Wasm {
        instance: Rc<Instance>,
        /// The index among the module's defined functions.
        index: u32,
        ty: FuncType,
        /// The JavaScript function that exports it, once made.
        export: RefCell<Option<Object>>,
    },
    Host {
        ty: FuncType,
        callable: Value,
        export: RefCell<Option<Object>>,
    },
}

impl Func {
    pub(crate) fn ty(&self) -> &FuncType {
        match self {
            Func::Wasm { ty, .. } | Func::Host { ty, .. } => ty,
        }
    }

    pub(crate) fn export(&self) -> &RefCell<Option<Object>> {
        match self {
            Func::Wasm { export, .. } | Func::Host { export, .. } => export,
        }
    }
}

/// A linear memory. Its bytes are shared with the `ArrayBuffer` its
/// `buffer` reads; growing makes a new buffer object over them.
#[derive(Debug)]
pub(crate) struct Memory {
    pub(crate) bytes: Bytes,
    pub(crate) limits: RefCell<Limits>,
    pub(crate) buffer: RefCell<Option<Object>>,
    /// The `WebAssembly.Memory` object, once made.
    pub(crate) object: RefCell<Option<Object>>,
}

impl Memory {
    pub(crate) fn new(limits: Limits) -> Option<Memory> {
        let size = limits.min as usize * PAGE;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).ok()?;
        bytes.resize(size, 0);
        Some(Memory {
            bytes: Rc::new(RefCell::new(bytes)),
            limits: RefCell::new(limits),
            buffer: RefCell::new(None),
            object: RefCell::new(None),
        })
    }

    pub(crate) fn pages(&self) -> u32 {
        (self.bytes.borrow().len() / PAGE) as u32
    }

    /// Grows the memory by `delta` pages, returning its previous size, or
    /// `None` when it cannot grow that far.
    pub(crate) fn grow(&self, delta: u32) -> Option<u32> {
        let old = self.pages();
        let max = self.limits.borrow().max.unwrap_or(MAX_PAGES).min(MAX_PAGES);
        let new = old.checked_add(delta).filter(|&new| new <= max)?;
        if delta > 0 {
            let mut bytes = self.bytes.borrow_mut();
            let size = new as usize * PAGE;
            let extra = size - bytes.len();
            bytes.try_reserve_exact(extra).ok()?;
            bytes.resize(size, 0);
            drop(bytes);
            *self.buffer.borrow_mut() = None;
        }
        Some(old)
    }
}

/// A table of references.
#[derive(Debug)]
pub(crate) struct Table {
    pub(crate) ty: TableType,
    pub(crate) elements: RefCell<Vec<Option<Ref>>>,
    /// The `WebAssembly.Table` object, once made.
    pub(crate) object: RefCell<Option<Object>>,
}

impl Table {
    pub(crate) fn new(ty: TableType, initial: Option<Ref>) -> Table {
        Table {
            ty,
            elements: RefCell::new(vec![initial; ty.limits.min as usize]),
            object: RefCell::new(None),
        }
    }

    pub(crate) fn size(&self) -> u32 {
        self.elements.borrow().len() as u32
    }

    /// Grows the table by `delta` elements set to `initial`, returning its
    /// previous size.
    pub(crate) fn grow(&self, delta: u32, initial: Option<Ref>) -> Option<u32> {
        let old = self.size();
        let max = self.ty.limits.max.unwrap_or(10_000_000).min(10_000_000);
        let new = old.checked_add(delta).filter(|&new| new <= max)?;
        let mut elements = self.elements.borrow_mut();
        elements.try_reserve(delta as usize).ok()?;
        elements.resize(new as usize, initial);
        Some(old)
    }
}

/// A global variable.
#[derive(Debug)]
pub(crate) struct Global {
    pub(crate) ty: GlobalType,
    pub(crate) value: RefCell<Val>,
    /// The `WebAssembly.Global` object, once made.
    pub(crate) object: RefCell<Option<Object>>,
}

impl Global {
    pub(crate) fn new(ty: GlobalType, value: Val) -> Global {
        Global {
            ty,
            value: RefCell::new(value),
            object: RefCell::new(None),
        }
    }
}

/// An import's or export's value.
#[derive(Clone, Debug)]
pub(crate) enum Extern {
    Func(Rc<Func>),
    Table(Rc<Table>),
    Memory(Rc<Memory>),
    Global(Rc<Global>),
}

/// A module instance.
#[derive(Debug)]
pub(crate) struct Instance {
    pub(crate) module: Rc<Module>,
    pub(crate) funcs: OnceCell<Vec<Rc<Func>>>,
    pub(crate) tables: Vec<Rc<Table>>,
    pub(crate) memory: Option<Rc<Memory>>,
    pub(crate) globals: OnceCell<Vec<Rc<Global>>>,
    /// Element segments' references; dropped segments are empty.
    pub(crate) elems: RefCell<Vec<Vec<Option<Ref>>>>,
    /// Which data segments are dropped.
    pub(crate) dropped: RefCell<Vec<bool>>,
}

impl Instance {
    pub(crate) fn funcs(&self) -> &[Rc<Func>] {
        self.funcs.get().map_or(&[], Vec::as_slice)
    }

    pub(crate) fn globals(&self) -> &[Rc<Global>] {
        self.globals.get().map_or(&[], Vec::as_slice)
    }

    /// An export's value.
    pub(crate) fn export(&self, kind: super::decode::ExternKind, index: u32) -> Extern {
        use super::decode::ExternKind;
        let index = index as usize;
        match kind {
            ExternKind::Func => Extern::Func(self.funcs()[index].clone()),
            ExternKind::Table => Extern::Table(self.tables[index].clone()),
            ExternKind::Memory => Extern::Memory(self.memory.clone().expect("a memory")),
            ExternKind::Global | ExternKind::Tag => Extern::Global(self.globals()[index].clone()),
        }
    }
}

/// Why execution stopped.
#[derive(Debug)]
pub(crate) enum Trap {
    Unreachable,
    MemoryOutOfBounds,
    DivideByZero,
    RemainderByZero,
    DivideUnrepresentable,
    FloatUnrepresentable,
    TableOutOfBounds,
    NullFunction,
    SignatureMismatch,
    DataOutOfBounds,
    StackOverflow,
    /// A JavaScript exception, passing through.
    Thrown(Thrown),
}

impl Trap {
    /// The `WebAssembly.RuntimeError` message, as V8 words it.
    pub(crate) fn message(&self) -> &'static str {
        match self {
            Trap::Unreachable => "unreachable",
            Trap::MemoryOutOfBounds => "memory access out of bounds",
            Trap::DivideByZero => "divide by zero",
            Trap::RemainderByZero => "remainder by zero",
            Trap::DivideUnrepresentable => "divide result unrepresentable",
            Trap::FloatUnrepresentable => "float unrepresentable in integer range",
            Trap::TableOutOfBounds => "table index is out of bounds",
            Trap::NullFunction | Trap::SignatureMismatch => {
                "null function or function signature mismatch"
            }
            Trap::DataOutOfBounds => "data segment out of bounds",
            Trap::StackOverflow => "Maximum call stack size exceeded",
            Trap::Thrown(_) => "",
        }
    }
}

/// Why instantiation failed.
#[derive(Debug)]
pub(crate) enum InstantiateError {
    Link(String),
    Trap(Trap),
}

/// Converts values between WebAssembly and the host: JavaScript values for
/// imported functions' arguments and results.
pub(crate) trait Host {
    /// Calls a host function with converted arguments.
    fn call(&mut self, callable: &Value, ty: &FuncType, args: Vec<Val>) -> Result<Vec<Val>, Trap>;
}

/// Instantiates a module with resolved imports, in import order. The
/// imports' kinds have been checked; their types are checked here.
pub(crate) fn instantiate(
    host: &mut dyn Host,
    module: Rc<Module>,
    imports: Vec<Extern>,
) -> Result<Rc<Instance>, InstantiateError> {
    let mut funcs = Vec::new();
    let mut tables = Vec::new();
    let mut memory = None;
    let mut globals = Vec::new();
    for (index, (import, value)) in module.imports.iter().zip(imports).enumerate() {
        let fail = |what: &str| {
            InstantiateError::Link(format!(
                "Import #{index} module=\"{}\" function=\"{}\": {what}",
                import.module, import.name
            ))
        };
        match (&import.desc, value) {
            (ImportDesc::Func(ty), Extern::Func(func)) => {
                if func.ty() != &module.types[*ty as usize] {
                    return Err(fail("imported function does not match the expected type"));
                }
                funcs.push(func);
            }
            (ImportDesc::Table(ty), Extern::Table(table)) => {
                if table.ty.element != ty.element {
                    return Err(fail("imported table does not match the expected type"));
                }
                let own = Limits {
                    min: table.size(),
                    max: table.ty.limits.max,
                };
                if !own.matches(&ty.limits) {
                    return Err(fail(if own.min < ty.limits.min {
                        "table import has a smaller initial size than required"
                    } else {
                        "table import has a larger maximum size than allowed"
                    }));
                }
                tables.push(table);
            }
            (ImportDesc::Memory(limits), Extern::Memory(imported)) => {
                let own = Limits {
                    min: imported.pages(),
                    max: imported.limits.borrow().max,
                };
                if !own.matches(limits) {
                    return Err(fail(if own.min < limits.min {
                        "memory import has a smaller initial size than required"
                    } else if own.max.is_none() {
                        "memory import has no maximum limit, expected at most a maximum"
                    } else {
                        "memory import has a larger maximum size than allowed"
                    }));
                }
                memory = Some(imported);
            }
            (ImportDesc::Global(ty), Extern::Global(global)) => {
                if global.ty != *ty {
                    return Err(fail(if global.ty.mutable != ty.mutable {
                        "imported global does not match the expected mutability"
                    } else {
                        "imported global does not match the expected type"
                    }));
                }
                globals.push(global);
            }
            (desc, _) => {
                return Err(fail(match desc {
                    ImportDesc::Func(_) => "function import requires a callable",
                    ImportDesc::Table(_) => "table import requires a WebAssembly.Table",
                    ImportDesc::Memory(_) => "memory import must be a WebAssembly.Memory object",
                    ImportDesc::Global(_) => {
                        "global import must be a number, valid Wasm reference, or WebAssembly.Global object"
                    }
                }));
            }
        }
    }
    for limits in &module.memories {
        memory = Some(Rc::new(Memory::new(*limits).ok_or_else(|| {
            InstantiateError::Link(
                "Out of memory: Cannot allocate Wasm memory for new instance".to_owned(),
            )
        })?));
    }
    for table in &module.tables {
        tables.push(Rc::new(Table::new(*table, None)));
    }
    let instance = Rc::new(Instance {
        module: module.clone(),
        funcs: OnceCell::new(),
        tables,
        memory,
        globals: OnceCell::new(),
        elems: RefCell::new(Vec::new()),
        dropped: RefCell::new(vec![false; module.datas.len()]),
    });
    for (index, &ty) in module.funcs.iter().enumerate() {
        funcs.push(Rc::new(Func::Wasm {
            instance: instance.clone(),
            index: index as u32,
            ty: module.types[ty as usize].clone(),
            export: RefCell::new(None),
        }));
    }
    let _ = instance.funcs.set(funcs);
    // Globals' initializers read imported and earlier globals.
    for global in &module.globals {
        let value = evaluate(&instance, &globals, &global.init);
        globals.push(Rc::new(Global::new(global.ty, value)));
    }
    let _ = instance.globals.set(globals);
    let mut elems = Vec::new();
    for elem in &module.elems {
        let items = elem
            .items
            .iter()
            .map(|item| match evaluate(&instance, instance.globals(), item) {
                Val::Ref(reference) => reference,
                _ => None,
            })
            .collect();
        elems.push(items);
    }
    *instance.elems.borrow_mut() = elems;
    for (index, elem) in module.elems.iter().enumerate() {
        match &elem.mode {
            SegmentMode::Active {
                index: table,
                offset,
            } => {
                let offset = match evaluate(&instance, instance.globals(), offset) {
                    Val::I32(offset) => offset as u32,
                    _ => 0,
                };
                let length = elem.items.len() as u32;
                table_init(&instance, *table, index as u32, offset, 0, length)
                    .map_err(InstantiateError::Trap)?;
                instance.elems.borrow_mut()[index].clear();
            }
            SegmentMode::Declarative => instance.elems.borrow_mut()[index].clear(),
            SegmentMode::Passive => {}
        }
    }
    for (index, data) in module.datas.iter().enumerate() {
        if let SegmentMode::Active { offset, .. } = &data.mode {
            let offset = match evaluate(&instance, instance.globals(), offset) {
                Val::I32(offset) => offset as u32,
                _ => 0,
            };
            let length = (data.bytes.end - data.bytes.start) as u32;
            memory_init(&instance, index as u32, offset, 0, length).map_err(|trap| {
                InstantiateError::Trap(match trap {
                    Trap::MemoryOutOfBounds => Trap::DataOutOfBounds,
                    trap => trap,
                })
            })?;
            instance.dropped.borrow_mut()[index] = true;
        }
    }
    if let Some(start) = module.start {
        let func = instance.funcs()[start as usize].clone();
        invoke(host, &func, Vec::new()).map_err(InstantiateError::Trap)?;
    }
    Ok(instance)
}

/// Evaluates a constant expression.
fn evaluate(instance: &Instance, globals: &[Rc<Global>], expr: &ConstExpr) -> Val {
    let mut stack: Vec<Val> = Vec::new();
    for op in expr {
        let value = match op {
            ConstOp::I32(value) => Val::I32(*value),
            ConstOp::I64(value) => Val::I64(*value),
            ConstOp::F32(bits) => Val::F32(*bits),
            ConstOp::F64(bits) => Val::F64(*bits),
            ConstOp::GlobalGet(index) => globals
                .get(*index as usize)
                .map_or(Val::I32(0), |global| global.value.borrow().clone()),
            ConstOp::RefNull(_) => Val::Ref(None),
            ConstOp::RefFunc(index) => Val::Ref(
                instance
                    .funcs()
                    .get(*index as usize)
                    .map(|func| Ref::Func(func.clone())),
            ),
            arithmetic => {
                let b = stack.pop();
                let a = stack.pop();
                match (arithmetic, a, b) {
                    (ConstOp::I32Add, Some(Val::I32(a)), Some(Val::I32(b))) => {
                        Val::I32(a.wrapping_add(b))
                    }
                    (ConstOp::I32Sub, Some(Val::I32(a)), Some(Val::I32(b))) => {
                        Val::I32(a.wrapping_sub(b))
                    }
                    (ConstOp::I32Mul, Some(Val::I32(a)), Some(Val::I32(b))) => {
                        Val::I32(a.wrapping_mul(b))
                    }
                    (ConstOp::I64Add, Some(Val::I64(a)), Some(Val::I64(b))) => {
                        Val::I64(a.wrapping_add(b))
                    }
                    (ConstOp::I64Sub, Some(Val::I64(a)), Some(Val::I64(b))) => {
                        Val::I64(a.wrapping_sub(b))
                    }
                    (ConstOp::I64Mul, Some(Val::I64(a)), Some(Val::I64(b))) => {
                        Val::I64(a.wrapping_mul(b))
                    }
                    _ => Val::I32(0),
                }
            }
        };
        stack.push(value);
    }
    stack.pop().unwrap_or(Val::I32(0))
}

fn table_init(
    instance: &Instance,
    table: u32,
    elem: u32,
    destination: u32,
    source: u32,
    length: u32,
) -> Result<(), Trap> {
    let table = &instance.tables[table as usize];
    let elems = instance.elems.borrow();
    let segment = &elems[elem as usize];
    let mut elements = table.elements.borrow_mut();
    let source_end = u64::from(source) + u64::from(length);
    let destination_end = u64::from(destination) + u64::from(length);
    if source_end > segment.len() as u64 || destination_end > elements.len() as u64 {
        return Err(Trap::TableOutOfBounds);
    }
    elements[destination as usize..destination_end as usize]
        .clone_from_slice(&segment[source as usize..source_end as usize]);
    Ok(())
}

fn memory_init(
    instance: &Instance,
    data: u32,
    destination: u32,
    source: u32,
    length: u32,
) -> Result<(), Trap> {
    let memory = instance.memory.as_ref().ok_or(Trap::MemoryOutOfBounds)?;
    let segment = &instance.module.datas[data as usize];
    let segment_bytes = if instance.dropped.borrow()[data as usize] {
        &[][..]
    } else {
        &instance.module.bytes[segment.bytes.clone()]
    };
    let mut bytes = memory.bytes.borrow_mut();
    let source_end = u64::from(source) + u64::from(length);
    let destination_end = u64::from(destination) + u64::from(length);
    if source_end > segment_bytes.len() as u64 || destination_end > bytes.len() as u64 {
        return Err(Trap::MemoryOutOfBounds);
    }
    bytes[destination as usize..destination_end as usize]
        .copy_from_slice(&segment_bytes[source as usize..source_end as usize]);
    Ok(())
}

/// Calls a function with arguments of its parameter types.
pub(crate) fn invoke(
    host: &mut dyn Host,
    func: &Rc<Func>,
    args: Vec<Val>,
) -> Result<Vec<Val>, Trap> {
    match &**func {
        Func::Host { ty, callable, .. } => host.call(callable, ty, args),
        Func::Wasm {
            instance,
            index,
            ty,
            ..
        } => {
            let mut exec = Exec {
                stack: Vec::new(),
                refs: Vec::new(),
                frames: Vec::new(),
            };
            for arg in args {
                let slot = exec.slot(arg);
                exec.stack.push(slot);
            }
            exec.run(host, instance.clone(), *index)?;
            let count = ty.results.len();
            let start = exec.stack.len() - count;
            let results = ty
                .results
                .iter()
                .zip(start..)
                .map(|(&ty, at)| exec.val(ty, exec.stack[at]))
                .collect();
            Ok(results)
        }
    }
}

struct Frame {
    instance: Rc<Instance>,
    body: Rc<Body>,
    pc: usize,
    base: usize,
}

/// One call into WebAssembly: its operand stack, the references its slots
/// name, and its suspended frames.
struct Exec {
    stack: Vec<u64>,
    refs: Vec<Ref>,
    frames: Vec<Frame>,
}

#[inline]
fn f32_of(bits: u64) -> f32 {
    f32::from_bits(bits as u32)
}

#[inline]
fn f64_of(bits: u64) -> f64 {
    f64::from_bits(bits)
}

#[inline]
fn of_f32(value: f32) -> u64 {
    u64::from(value.to_bits())
}

#[inline]
fn of_i32(value: i32) -> u64 {
    u64::from(value as u32)
}

#[inline]
fn of_bool(value: bool) -> u64 {
    u64::from(value)
}

const F32_CANONICAL_NAN: u32 = 0x7FC0_0000;
const F64_CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;

fn f64_trunc(value: f64) -> f64 {
    crate::libm::trunc(value)
}

/// Rounds to the nearest integer, ties to even.
fn f64_nearest(value: f64) -> f64 {
    let magnitude = value.abs();
    if magnitude.is_nan() || magnitude >= 4_503_599_627_370_496.0 {
        // Integral, infinite or NaN.
        return value;
    }
    let rounded = (magnitude + 4_503_599_627_370_496.0) - 4_503_599_627_370_496.0;
    if value.is_sign_negative() {
        -rounded
    } else {
        rounded
    }
}

fn f32_nearest(value: f32) -> f32 {
    let magnitude = value.abs();
    if magnitude.is_nan() || magnitude >= 8_388_608.0 {
        return value;
    }
    let rounded = (magnitude + 8_388_608.0) - 8_388_608.0;
    if value.is_sign_negative() {
        -rounded
    } else {
        rounded
    }
}

fn f64_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::from_bits(F64_CANONICAL_NAN)
    } else if a == b {
        // -0 is less than +0.
        f64::from_bits(a.to_bits() | b.to_bits())
    } else if a < b {
        a
    } else {
        b
    }
}

fn f64_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::from_bits(F64_CANONICAL_NAN)
    } else if a == b {
        f64::from_bits(a.to_bits() & b.to_bits())
    } else if a > b {
        a
    } else {
        b
    }
}

fn f32_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::from_bits(F32_CANONICAL_NAN)
    } else if a == b {
        f32::from_bits(a.to_bits() | b.to_bits())
    } else if a < b {
        a
    } else {
        b
    }
}

fn f32_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::from_bits(F32_CANONICAL_NAN)
    } else if a == b {
        f32::from_bits(a.to_bits() & b.to_bits())
    } else if a > b {
        a
    } else {
        b
    }
}

/// Truncates toward zero, trapping outside `[low, high]` (after
/// truncation) or for NaN.
fn truncate(value: f64, low: f64, high: f64) -> Result<f64, Trap> {
    let truncated = f64_trunc(value);
    if truncated >= low && truncated <= high {
        Ok(truncated)
    } else {
        Err(Trap::FloatUnrepresentable)
    }
}

/// Runs a numeric instruction on the top of the stack.
#[inline]
fn numeric(stack: &mut Vec<u64>, code: u16) -> Result<(), Trap> {
    macro_rules! unary {
        (|$a:ident| $e:expr) => {{
            let top = stack.last_mut().expect("an operand");
            let $a = *top;
            *top = $e;
        }};
    }
    macro_rules! binary {
        (|$a:ident, $b:ident| $e:expr) => {{
            let $b = stack.pop().expect("an operand");
            let top = stack.last_mut().expect("an operand");
            let $a = *top;
            *top = $e;
        }};
    }
    macro_rules! binary_try {
        (|$a:ident, $b:ident| $e:expr) => {{
            let $b = stack.pop().expect("an operand");
            let top = stack.last_mut().expect("an operand");
            let $a = *top;
            *top = $e?;
        }};
    }
    match code {
        0x45 => unary!(|a| of_bool(a as u32 == 0)),
        0x46 => binary!(|a, b| of_bool(a as u32 == b as u32)),
        0x47 => binary!(|a, b| of_bool(a as u32 != b as u32)),
        0x48 => binary!(|a, b| of_bool((a as i32) < (b as i32))),
        0x49 => binary!(|a, b| of_bool((a as u32) < (b as u32))),
        0x4A => binary!(|a, b| of_bool((a as i32) > (b as i32))),
        0x4B => binary!(|a, b| of_bool((a as u32) > (b as u32))),
        0x4C => binary!(|a, b| of_bool((a as i32) <= (b as i32))),
        0x4D => binary!(|a, b| of_bool((a as u32) <= (b as u32))),
        0x4E => binary!(|a, b| of_bool((a as i32) >= (b as i32))),
        0x4F => binary!(|a, b| of_bool((a as u32) >= (b as u32))),
        0x50 => unary!(|a| of_bool(a == 0)),
        0x51 => binary!(|a, b| of_bool(a == b)),
        0x52 => binary!(|a, b| of_bool(a != b)),
        0x53 => binary!(|a, b| of_bool((a as i64) < (b as i64))),
        0x54 => binary!(|a, b| of_bool(a < b)),
        0x55 => binary!(|a, b| of_bool((a as i64) > (b as i64))),
        0x56 => binary!(|a, b| of_bool(a > b)),
        0x57 => binary!(|a, b| of_bool((a as i64) <= (b as i64))),
        0x58 => binary!(|a, b| of_bool(a <= b)),
        0x59 => binary!(|a, b| of_bool((a as i64) >= (b as i64))),
        0x5A => binary!(|a, b| of_bool(a >= b)),
        0x5B => binary!(|a, b| of_bool(f32_of(a) == f32_of(b))),
        0x5C => binary!(|a, b| of_bool(f32_of(a) != f32_of(b))),
        0x5D => binary!(|a, b| of_bool(f32_of(a) < f32_of(b))),
        0x5E => binary!(|a, b| of_bool(f32_of(a) > f32_of(b))),
        0x5F => binary!(|a, b| of_bool(f32_of(a) <= f32_of(b))),
        0x60 => binary!(|a, b| of_bool(f32_of(a) >= f32_of(b))),
        0x61 => binary!(|a, b| of_bool(f64_of(a) == f64_of(b))),
        0x62 => binary!(|a, b| of_bool(f64_of(a) != f64_of(b))),
        0x63 => binary!(|a, b| of_bool(f64_of(a) < f64_of(b))),
        0x64 => binary!(|a, b| of_bool(f64_of(a) > f64_of(b))),
        0x65 => binary!(|a, b| of_bool(f64_of(a) <= f64_of(b))),
        0x66 => binary!(|a, b| of_bool(f64_of(a) >= f64_of(b))),
        0x67 => unary!(|a| u64::from((a as u32).leading_zeros())),
        0x68 => unary!(|a| u64::from((a as u32).trailing_zeros())),
        0x69 => unary!(|a| u64::from((a as u32).count_ones())),
        0x6A => binary!(|a, b| u64::from((a as u32).wrapping_add(b as u32))),
        0x6B => binary!(|a, b| u64::from((a as u32).wrapping_sub(b as u32))),
        0x6C => binary!(|a, b| u64::from((a as u32).wrapping_mul(b as u32))),
        0x6D => binary_try!(|a, b| {
            let (a, b) = (a as i32, b as i32);
            if b == 0 {
                Err(Trap::DivideByZero)
            } else if a == i32::MIN && b == -1 {
                Err(Trap::DivideUnrepresentable)
            } else {
                Ok(of_i32(a / b))
            }
        }),
        0x6E => binary_try!(|a, b| {
            (a as u32)
                .checked_div(b as u32)
                .map(u64::from)
                .ok_or(Trap::DivideByZero)
        }),
        0x6F => binary_try!(|a, b| {
            let (a, b) = (a as i32, b as i32);
            if b == 0 {
                Err(Trap::RemainderByZero)
            } else {
                Ok(of_i32(a.wrapping_rem(b)))
            }
        }),
        0x70 => binary_try!(|a, b| {
            let (a, b) = (a as u32, b as u32);
            if b == 0 {
                Err(Trap::RemainderByZero)
            } else {
                Ok(u64::from(a % b))
            }
        }),
        0x71 => binary!(|a, b| a & b),
        0x72 => binary!(|a, b| a | b),
        0x73 => binary!(|a, b| a ^ b),
        0x74 => binary!(|a, b| u64::from((a as u32).wrapping_shl(b as u32))),
        0x75 => binary!(|a, b| of_i32((a as i32).wrapping_shr(b as u32))),
        0x76 => binary!(|a, b| u64::from((a as u32).wrapping_shr(b as u32))),
        0x77 => binary!(|a, b| u64::from((a as u32).rotate_left(b as u32 % 32))),
        0x78 => binary!(|a, b| u64::from((a as u32).rotate_right(b as u32 % 32))),
        0x79 => unary!(|a| u64::from(a.leading_zeros())),
        0x7A => unary!(|a| u64::from(a.trailing_zeros())),
        0x7B => unary!(|a| u64::from(a.count_ones())),
        0x7C => binary!(|a, b| a.wrapping_add(b)),
        0x7D => binary!(|a, b| a.wrapping_sub(b)),
        0x7E => binary!(|a, b| a.wrapping_mul(b)),
        0x7F => binary_try!(|a, b| {
            let (a, b) = (a as i64, b as i64);
            if b == 0 {
                Err(Trap::DivideByZero)
            } else if a == i64::MIN && b == -1 {
                Err(Trap::DivideUnrepresentable)
            } else {
                Ok((a / b) as u64)
            }
        }),
        0x80 => binary_try!(|a, b| a.checked_div(b).ok_or(Trap::DivideByZero)),
        0x81 => binary_try!(|a, b| {
            let (a, b) = (a as i64, b as i64);
            if b == 0 {
                Err(Trap::RemainderByZero)
            } else {
                Ok(a.wrapping_rem(b) as u64)
            }
        }),
        0x82 => binary_try!(|a, b| if b == 0 {
            Err(Trap::RemainderByZero)
        } else {
            Ok(a % b)
        }),
        0x83 => binary!(|a, b| a & b),
        0x84 => binary!(|a, b| a | b),
        0x85 => binary!(|a, b| a ^ b),
        0x86 => binary!(|a, b| a.wrapping_shl(b as u32)),
        0x87 => binary!(|a, b| (a as i64).wrapping_shr(b as u32) as u64),
        0x88 => binary!(|a, b| a.wrapping_shr(b as u32)),
        0x89 => binary!(|a, b| a.rotate_left((b % 64) as u32)),
        0x8A => binary!(|a, b| a.rotate_right((b % 64) as u32)),
        0x8B => unary!(|a| a & 0x7FFF_FFFF),
        0x8C => unary!(|a| (a ^ 0x8000_0000) & 0xFFFF_FFFF),
        0x8D => unary!(|a| of_f32(crate::libm::ceil(f64::from(f32_of(a))) as f32)),
        0x8E => unary!(|a| of_f32(crate::libm::floor(f64::from(f32_of(a))) as f32)),
        0x8F => unary!(|a| of_f32(f64_trunc(f64::from(f32_of(a))) as f32)),
        0x90 => unary!(|a| of_f32(f32_nearest(f32_of(a)))),
        0x91 => unary!(|a| of_f32(crate::libm::sqrt(f64::from(f32_of(a))) as f32)),
        0x92 => binary!(|a, b| of_f32(f32_of(a) + f32_of(b))),
        0x93 => binary!(|a, b| of_f32(f32_of(a) - f32_of(b))),
        0x94 => binary!(|a, b| of_f32(f32_of(a) * f32_of(b))),
        0x95 => binary!(|a, b| of_f32(f32_of(a) / f32_of(b))),
        0x96 => binary!(|a, b| of_f32(f32_min(f32_of(a), f32_of(b)))),
        0x97 => binary!(|a, b| of_f32(f32_max(f32_of(a), f32_of(b)))),
        0x98 => binary!(|a, b| (a & 0x7FFF_FFFF) | (b & 0x8000_0000)),
        0x99 => unary!(|a| a & 0x7FFF_FFFF_FFFF_FFFF),
        0x9A => unary!(|a| a ^ 0x8000_0000_0000_0000),
        0x9B => unary!(|a| crate::libm::ceil(f64_of(a)).to_bits()),
        0x9C => unary!(|a| crate::libm::floor(f64_of(a)).to_bits()),
        0x9D => unary!(|a| f64_trunc(f64_of(a)).to_bits()),
        0x9E => unary!(|a| f64_nearest(f64_of(a)).to_bits()),
        0x9F => unary!(|a| crate::libm::sqrt(f64_of(a)).to_bits()),
        0xA0 => binary!(|a, b| (f64_of(a) + f64_of(b)).to_bits()),
        0xA1 => binary!(|a, b| (f64_of(a) - f64_of(b)).to_bits()),
        0xA2 => binary!(|a, b| (f64_of(a) * f64_of(b)).to_bits()),
        0xA3 => binary!(|a, b| (f64_of(a) / f64_of(b)).to_bits()),
        0xA4 => binary!(|a, b| f64_min(f64_of(a), f64_of(b)).to_bits()),
        0xA5 => binary!(|a, b| f64_max(f64_of(a), f64_of(b)).to_bits()),
        0xA6 => binary!(|a, b| (a & 0x7FFF_FFFF_FFFF_FFFF) | (b & 0x8000_0000_0000_0000)),
        0xA7 => unary!(|a| a & 0xFFFF_FFFF),
        0xA8 | 0xAA => {
            let top = stack.last_mut().expect("an operand");
            let value = if code == 0xA8 {
                f64::from(f32_of(*top))
            } else {
                f64_of(*top)
            };
            *top = of_i32(truncate(value, -2_147_483_648.0, 2_147_483_647.0)? as i32);
        }
        0xA9 | 0xAB => {
            let top = stack.last_mut().expect("an operand");
            let value = if code == 0xA9 {
                f64::from(f32_of(*top))
            } else {
                f64_of(*top)
            };
            *top = u64::from(truncate(value, 0.0, 4_294_967_295.0)? as u32);
        }
        0xAC => unary!(|a| i64::from(a as i32) as u64),
        0xAD => unary!(|a| a & 0xFFFF_FFFF),
        0xAE | 0xB0 => {
            let top = stack.last_mut().expect("an operand");
            let value = if code == 0xAE {
                f64::from(f32_of(*top))
            } else {
                f64_of(*top)
            };
            let truncated = f64_trunc(value);
            let fits =
                (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&truncated);
            if !fits {
                return Err(Trap::FloatUnrepresentable);
            }
            *top = truncated as i64 as u64;
        }
        0xAF | 0xB1 => {
            let top = stack.last_mut().expect("an operand");
            let value = if code == 0xAF {
                f64::from(f32_of(*top))
            } else {
                f64_of(*top)
            };
            let truncated = f64_trunc(value);
            let fits = (0.0..18_446_744_073_709_551_616.0).contains(&truncated);
            if !fits {
                return Err(Trap::FloatUnrepresentable);
            }
            *top = truncated as u64;
        }
        0xB2 => unary!(|a| of_f32(a as i32 as f32)),
        0xB3 => unary!(|a| of_f32(a as u32 as f32)),
        0xB4 => unary!(|a| of_f32(a as i64 as f32)),
        0xB5 => unary!(|a| of_f32(a as f32)),
        0xB6 => unary!(|a| of_f32(f64_of(a) as f32)),
        0xB7 => unary!(|a| f64::from(a as i32).to_bits()),
        0xB8 => unary!(|a| f64::from(a as u32).to_bits()),
        0xB9 => unary!(|a| (a as i64 as f64).to_bits()),
        0xBA => unary!(|a| (a as f64).to_bits()),
        0xBB => unary!(|a| f64::from(f32_of(a)).to_bits()),
        // Reinterpretations keep the bits.
        0xBC..=0xBF => {}
        0xC0 => unary!(|a| of_i32(i32::from(a as i8))),
        0xC1 => unary!(|a| of_i32(i32::from(a as i16))),
        0xC2 => unary!(|a| i64::from(a as i8) as u64),
        0xC3 => unary!(|a| i64::from(a as i16) as u64),
        0xC4 => unary!(|a| i64::from(a as i32) as u64),
        // The saturating truncations: Rust's casts saturate, and NaN
        // becomes zero.
        0x100 => unary!(|a| of_i32(f32_of(a) as i32)),
        0x101 => unary!(|a| u64::from(f32_of(a) as u32)),
        0x102 => unary!(|a| of_i32(f64_of(a) as i32)),
        0x103 => unary!(|a| u64::from(f64_of(a) as u32)),
        0x104 => unary!(|a| f32_of(a) as i64 as u64),
        0x105 => unary!(|a| f32_of(a) as u64),
        0x106 => unary!(|a| f64_of(a) as i64 as u64),
        0x107 => unary!(|a| f64_of(a) as u64),
        _ => unreachable!("compiled numeric opcodes"),
    }
    Ok(())
}

impl Exec {
    /// A value's slot.
    fn slot(&mut self, value: Val) -> u64 {
        match value {
            Val::I32(value) => of_i32(value),
            Val::I64(value) => value as u64,
            Val::F32(bits) => u64::from(bits),
            Val::F64(bits) => bits,
            Val::Ref(None) => 0,
            Val::Ref(Some(reference)) => self.reference(reference),
        }
    }

    fn reference(&mut self, reference: Ref) -> u64 {
        self.refs.push(reference);
        self.refs.len() as u64
    }

    fn reference_at(&self, slot: u64) -> Option<Ref> {
        (slot != 0).then(|| self.refs[slot as usize - 1].clone())
    }

    /// A slot's value.
    fn val(&self, ty: ValType, slot: u64) -> Val {
        match ty {
            ValType::I32 => Val::I32(slot as i32),
            ValType::I64 => Val::I64(slot as i64),
            ValType::F32 => Val::F32(slot as u32),
            ValType::F64 => Val::F64(slot),
            ValType::FuncRef | ValType::ExternRef => Val::Ref(self.reference_at(slot)),
        }
    }

    /// Moves a branch's values down to the target height.
    #[inline]
    fn branch(&mut self, to: usize, arity: usize) {
        let length = self.stack.len();
        if length - arity != to {
            self.stack.copy_within(length - arity..length, to);
            self.stack.truncate(to + arity);
        }
    }

    #[inline]
    fn address(&mut self, offset: u32) -> usize {
        let address = self.stack.pop().expect("an address") as u32;
        address as usize + offset as usize
    }

    fn run(&mut self, host: &mut dyn Host, instance: Rc<Instance>, index: u32) -> Result<(), Trap> {
        let mut instance = instance;
        let mut body = instance.module.bodies[index as usize].clone();
        let mut base = self.stack.len() - body.params as usize;
        self.stack.resize(base + body.locals as usize, 0);
        let mut operands = base + body.locals as usize;
        let mut memory = instance.memory.clone();
        let mut pc = 0usize;
        macro_rules! load {
            ($offset:expr, $size:expr, |$bytes:ident| $convert:expr) => {{
                let address = self.address($offset);
                let memory = memory.as_ref().expect("a memory");
                let all = memory.bytes.borrow();
                let Some(slice) = all.get(address..address + $size) else {
                    return Err(Trap::MemoryOutOfBounds);
                };
                let $bytes: [u8; $size] = slice.try_into().expect("the access size");
                let value = $convert;
                drop(all);
                self.stack.push(value);
            }};
        }
        macro_rules! store {
            ($offset:expr, $size:expr, |$value:ident| $bytes:expr) => {{
                let $value = self.stack.pop().expect("a value");
                let address = self.address($offset);
                let memory = memory.as_ref().expect("a memory");
                let mut all = memory.bytes.borrow_mut();
                let Some(slice) = all.get_mut(address..address + $size) else {
                    return Err(Trap::MemoryOutOfBounds);
                };
                let bytes: [u8; $size] = $bytes;
                slice.copy_from_slice(&bytes);
            }};
        }
        loop {
            let op = body.ops[pc];
            pc += 1;
            match op {
                Op::Unreachable => return Err(Trap::Unreachable),
                Op::Jump(target) => pc = target as usize,
                Op::JumpIfZero(target) => {
                    if self.stack.pop().expect("a condition") as u32 == 0 {
                        pc = target as usize;
                    }
                }
                Op::Br(Target {
                    pc: target,
                    height,
                    arity,
                }) => {
                    self.branch(operands + height as usize, arity as usize);
                    pc = target as usize;
                }
                Op::BrIf(Target {
                    pc: target,
                    height,
                    arity,
                }) => {
                    if self.stack.pop().expect("a condition") as u32 != 0 {
                        self.branch(operands + height as usize, arity as usize);
                        pc = target as usize;
                    }
                }
                Op::BrTable { start, count } => {
                    let index = self.stack.pop().expect("an index") as u32;
                    let chosen = index.min(count - 1);
                    let target = body.targets[(start + chosen) as usize];
                    self.branch(operands + target.height as usize, target.arity as usize);
                    pc = target.pc as usize;
                }
                Op::Return => {
                    let results = body.results as usize;
                    let length = self.stack.len();
                    self.stack.copy_within(length - results..length, base);
                    self.stack.truncate(base + results);
                    let Some(frame) = self.frames.pop() else {
                        return Ok(());
                    };
                    instance = frame.instance;
                    body = frame.body;
                    pc = frame.pc;
                    base = frame.base;
                    operands = base + body.locals as usize;
                    memory = instance.memory.clone();
                }
                Op::Call(callee) => {
                    let func = instance.funcs()[callee as usize].clone();
                    match &*func {
                        Func::Wasm {
                            instance: callee,
                            index,
                            ..
                        } => {
                            if self.frames.len() >= MAX_FRAMES {
                                return Err(Trap::StackOverflow);
                            }
                            let callee_body = callee.module.bodies[*index as usize].clone();
                            let callee_base = self.stack.len() - callee_body.params as usize;
                            let top = callee_base + callee_body.locals as usize;
                            if top + callee_body.max_height as usize > MAX_SLOTS {
                                return Err(Trap::StackOverflow);
                            }
                            self.frames.push(Frame {
                                instance: core::mem::replace(&mut instance, callee.clone()),
                                body: core::mem::replace(&mut body, callee_body),
                                pc,
                                base,
                            });
                            base = callee_base;
                            self.stack.resize(top, 0);
                            operands = top;
                            memory = instance.memory.clone();
                            pc = 0;
                        }
                        Func::Host { ty, callable, .. } => {
                            self.call_host(host, callable, ty)?;
                        }
                    }
                }
                Op::CallIndirect { ty, table } => {
                    let element = self.stack.pop().expect("an element index") as u32;
                    let func = {
                        let elements = instance.tables[table as usize].elements.borrow();
                        match elements.get(element as usize) {
                            None => return Err(Trap::TableOutOfBounds),
                            Some(None) => return Err(Trap::NullFunction),
                            Some(Some(Ref::Func(func))) => func.clone(),
                            Some(Some(Ref::Extern(_))) => return Err(Trap::SignatureMismatch),
                        }
                    };
                    if func.ty() != &instance.module.types[ty as usize] {
                        return Err(Trap::SignatureMismatch);
                    }
                    match &*func {
                        Func::Wasm {
                            instance: callee,
                            index,
                            ..
                        } => {
                            if self.frames.len() >= MAX_FRAMES {
                                return Err(Trap::StackOverflow);
                            }
                            let callee_body = callee.module.bodies[*index as usize].clone();
                            let callee_base = self.stack.len() - callee_body.params as usize;
                            let top = callee_base + callee_body.locals as usize;
                            if top + callee_body.max_height as usize > MAX_SLOTS {
                                return Err(Trap::StackOverflow);
                            }
                            self.frames.push(Frame {
                                instance: core::mem::replace(&mut instance, callee.clone()),
                                body: core::mem::replace(&mut body, callee_body),
                                pc,
                                base,
                            });
                            base = callee_base;
                            self.stack.resize(top, 0);
                            operands = top;
                            memory = instance.memory.clone();
                            pc = 0;
                        }
                        Func::Host { ty, callable, .. } => {
                            self.call_host(host, callable, ty)?;
                        }
                    }
                }
                Op::Drop => {
                    self.stack.pop();
                }
                Op::Select => {
                    let condition = self.stack.pop().expect("a condition") as u32;
                    let second = self.stack.pop().expect("an operand");
                    if condition == 0 {
                        *self.stack.last_mut().expect("an operand") = second;
                    }
                }
                Op::LocalGet(index) => {
                    let value = self.stack[base + index as usize];
                    self.stack.push(value);
                }
                Op::LocalSet(index) => {
                    let value = self.stack.pop().expect("a value");
                    self.stack[base + index as usize] = value;
                }
                Op::LocalTee(index) => {
                    let value = *self.stack.last().expect("a value");
                    self.stack[base + index as usize] = value;
                }
                Op::GlobalGet(index) => {
                    let global = &instance.globals()[index as usize];
                    let value = global.value.borrow().clone();
                    let slot = self.slot(value);
                    self.stack.push(slot);
                }
                Op::GlobalSet(index) => {
                    let slot = self.stack.pop().expect("a value");
                    let global = &instance.globals()[index as usize];
                    let value = self.val(global.ty.ty, slot);
                    *global.value.borrow_mut() = value;
                }
                Op::TableGet(table) => {
                    let element = self.stack.pop().expect("an index") as u32;
                    let reference = {
                        let elements = instance.tables[table as usize].elements.borrow();
                        elements
                            .get(element as usize)
                            .cloned()
                            .ok_or(Trap::TableOutOfBounds)?
                    };
                    let slot = self.slot(Val::Ref(reference));
                    self.stack.push(slot);
                }
                Op::TableSet(table) => {
                    let slot = self.stack.pop().expect("a reference");
                    let element = self.stack.pop().expect("an index") as u32;
                    let reference = self.reference_at(slot);
                    let mut elements = instance.tables[table as usize].elements.borrow_mut();
                    let Some(entry) = elements.get_mut(element as usize) else {
                        return Err(Trap::TableOutOfBounds);
                    };
                    *entry = reference;
                }
                Op::I32Load(offset) => load!(offset, 4, |b| u64::from(u32::from_le_bytes(b))),
                Op::I64Load(offset) => load!(offset, 8, |b| u64::from_le_bytes(b)),
                Op::F32Load(offset) => load!(offset, 4, |b| u64::from(u32::from_le_bytes(b))),
                Op::F64Load(offset) => load!(offset, 8, |b| u64::from_le_bytes(b)),
                Op::I32Load8S(offset) => load!(offset, 1, |b| of_i32(i32::from(b[0] as i8))),
                Op::I32Load8U(offset) => load!(offset, 1, |b| u64::from(b[0])),
                Op::I32Load16S(offset) => {
                    load!(offset, 2, |b| of_i32(i32::from(i16::from_le_bytes(b))))
                }
                Op::I32Load16U(offset) => load!(offset, 2, |b| u64::from(u16::from_le_bytes(b))),
                Op::I64Load8S(offset) => load!(offset, 1, |b| i64::from(b[0] as i8) as u64),
                Op::I64Load8U(offset) => load!(offset, 1, |b| u64::from(b[0])),
                Op::I64Load16S(offset) => {
                    load!(offset, 2, |b| i64::from(i16::from_le_bytes(b)) as u64)
                }
                Op::I64Load16U(offset) => load!(offset, 2, |b| u64::from(u16::from_le_bytes(b))),
                Op::I64Load32S(offset) => {
                    load!(offset, 4, |b| i64::from(i32::from_le_bytes(b)) as u64)
                }
                Op::I64Load32U(offset) => load!(offset, 4, |b| u64::from(u32::from_le_bytes(b))),
                Op::I32Store(offset) | Op::F32Store(offset) => {
                    store!(offset, 4, |v| (v as u32).to_le_bytes())
                }
                Op::I64Store(offset) | Op::F64Store(offset) => {
                    store!(offset, 8, |v| v.to_le_bytes())
                }
                Op::I32Store8(offset) | Op::I64Store8(offset) => store!(offset, 1, |v| [v as u8]),
                Op::I32Store16(offset) | Op::I64Store16(offset) => {
                    store!(offset, 2, |v| (v as u16).to_le_bytes())
                }
                Op::I64Store32(offset) => store!(offset, 4, |v| (v as u32).to_le_bytes()),
                Op::MemorySize => {
                    let pages = memory.as_ref().map_or(0, |memory| memory.pages());
                    self.stack.push(u64::from(pages));
                }
                Op::MemoryGrow => {
                    let delta = self.stack.pop().expect("a delta") as u32;
                    let result = memory
                        .as_ref()
                        .and_then(|memory| memory.grow(delta))
                        .map_or(u64::from(u32::MAX), u64::from);
                    self.stack.push(result);
                }
                Op::Const(bits) => self.stack.push(bits),
                Op::Numeric(code) => numeric(&mut self.stack, code)?,
                Op::RefNull => self.stack.push(0),
                Op::RefIsNull => {
                    let top = self.stack.last_mut().expect("a reference");
                    *top = of_bool(*top == 0);
                }
                Op::RefFunc(index) => {
                    let func = instance.funcs()[index as usize].clone();
                    let slot = self.reference(Ref::Func(func));
                    self.stack.push(slot);
                }
                Op::MemoryInit(data) => {
                    let length = self.stack.pop().expect("a length") as u32;
                    let source = self.stack.pop().expect("a source") as u32;
                    let destination = self.stack.pop().expect("a destination") as u32;
                    memory_init(&instance, data, destination, source, length)?;
                }
                Op::DataDrop(data) => instance.dropped.borrow_mut()[data as usize] = true,
                Op::MemoryCopy => {
                    let length = self.stack.pop().expect("a length") as u32 as usize;
                    let source = self.stack.pop().expect("a source") as u32 as usize;
                    let destination = self.stack.pop().expect("a destination") as u32 as usize;
                    let memory = memory.as_ref().expect("a memory");
                    let mut bytes = memory.bytes.borrow_mut();
                    if source + length > bytes.len() || destination + length > bytes.len() {
                        return Err(Trap::MemoryOutOfBounds);
                    }
                    bytes.copy_within(source..source + length, destination);
                }
                Op::MemoryFill => {
                    let length = self.stack.pop().expect("a length") as u32 as usize;
                    let value = self.stack.pop().expect("a value") as u8;
                    let destination = self.stack.pop().expect("a destination") as u32 as usize;
                    let memory = memory.as_ref().expect("a memory");
                    let mut bytes = memory.bytes.borrow_mut();
                    let Some(range) = bytes.get_mut(destination..destination + length) else {
                        return Err(Trap::MemoryOutOfBounds);
                    };
                    range.fill(value);
                }
                Op::TableInit { elem, table } => {
                    let length = self.stack.pop().expect("a length") as u32;
                    let source = self.stack.pop().expect("a source") as u32;
                    let destination = self.stack.pop().expect("a destination") as u32;
                    table_init(&instance, table, elem, destination, source, length)?;
                }
                Op::ElemDrop(elem) => instance.elems.borrow_mut()[elem as usize].clear(),
                Op::TableCopy {
                    destination,
                    source,
                } => {
                    let length = self.stack.pop().expect("a length") as u32 as usize;
                    let from = self.stack.pop().expect("a source") as u32 as usize;
                    let to = self.stack.pop().expect("a destination") as u32 as usize;
                    let source_table = &instance.tables[source as usize];
                    let destination_table = &instance.tables[destination as usize];
                    let copied: Vec<Option<Ref>> = {
                        let elements = source_table.elements.borrow();
                        if from + length > elements.len() {
                            return Err(Trap::TableOutOfBounds);
                        }
                        elements[from..from + length].to_vec()
                    };
                    let mut elements = destination_table.elements.borrow_mut();
                    if to + length > elements.len() {
                        return Err(Trap::TableOutOfBounds);
                    }
                    elements[to..to + length].clone_from_slice(&copied);
                }
                Op::TableGrow(table) => {
                    let delta = self.stack.pop().expect("a delta") as u32;
                    let slot = self.stack.pop().expect("a reference");
                    let initial = self.reference_at(slot);
                    let result = instance.tables[table as usize]
                        .grow(delta, initial)
                        .map_or(u64::from(u32::MAX), u64::from);
                    self.stack.push(result);
                }
                Op::TableSize(table) => {
                    let size = instance.tables[table as usize].size();
                    self.stack.push(u64::from(size));
                }
                Op::TableFill(table) => {
                    let length = self.stack.pop().expect("a length") as u32 as usize;
                    let slot = self.stack.pop().expect("a reference");
                    let destination = self.stack.pop().expect("a destination") as u32 as usize;
                    let reference = self.reference_at(slot);
                    let mut elements = instance.tables[table as usize].elements.borrow_mut();
                    let Some(range) = elements.get_mut(destination..destination + length) else {
                        return Err(Trap::TableOutOfBounds);
                    };
                    for entry in range {
                        *entry = reference.clone();
                    }
                }
            }
        }
    }

    /// Calls a host function with the arguments on the stack, pushing its
    /// results.
    fn call_host(
        &mut self,
        host: &mut dyn Host,
        callable: &Value,
        ty: &FuncType,
    ) -> Result<(), Trap> {
        let start = self.stack.len() - ty.params.len();
        let args = ty
            .params
            .iter()
            .zip(start..)
            .map(|(&param, at)| self.val(param, self.stack[at]))
            .collect();
        self.stack.truncate(start);
        let results = host.call(callable, ty, args)?;
        for value in results {
            let slot = self.slot(value);
            self.stack.push(slot);
        }
        Ok(())
    }
}

/// The host functions of a realm: imported JavaScript functions, called
/// with converted arguments.
pub(crate) struct RealmHost<'a> {
    pub(crate) realm: &'a mut Realm,
}

impl Host for RealmHost<'_> {
    fn call(&mut self, callable: &Value, ty: &FuncType, args: Vec<Val>) -> Result<Vec<Val>, Trap> {
        crate::wasm_binding::call_host(self.realm, callable, ty, args)
    }
}
