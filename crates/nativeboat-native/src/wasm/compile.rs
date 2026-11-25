//! Function bodies, compiled from WebAssembly code into the instructions
//! [`super::exec`] runs: structured control becomes jumps whose targets and
//! operand-stack heights are resolved here, so execution keeps no control
//! stack. Compiling also checks each instruction's indexes and the operand
//! stack's depth.

#[allow(unused_imports)]
use crate::prelude::*;

use super::decode::{CompileError, Decoded, ExternKind, Module, Reader};
use super::{FuncType, GlobalType, TableType, ValType};

/// Where a branch goes: the instruction, and the operand-stack height above
/// the frame's locals that the branch's values move down to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) pc: u32,
    pub(crate) height: u32,
    pub(crate) arity: u32,
}

/// One instruction, with its immediates resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    Unreachable,
    Jump(u32),
    JumpIfZero(u32),
    Br(Target),
    BrIf(Target),
    /// Targets `start..start + count` in the body's table; the last is the
    /// default.
    BrTable {
        start: u32,
        count: u32,
    },
    Return,
    Call(u32),
    CallIndirect {
        ty: u32,
        table: u32,
    },
    Drop,
    Select,
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    GlobalGet(u32),
    GlobalSet(u32),
    TableGet(u32),
    TableSet(u32),
    I32Load(u32),
    I64Load(u32),
    F32Load(u32),
    F64Load(u32),
    I32Load8S(u32),
    I32Load8U(u32),
    I32Load16S(u32),
    I32Load16U(u32),
    I64Load8S(u32),
    I64Load8U(u32),
    I64Load16S(u32),
    I64Load16U(u32),
    I64Load32S(u32),
    I64Load32U(u32),
    I32Store(u32),
    I64Store(u32),
    F32Store(u32),
    F64Store(u32),
    I32Store8(u32),
    I32Store16(u32),
    I64Store8(u32),
    I64Store16(u32),
    I64Store32(u32),
    MemorySize,
    MemoryGrow,
    /// A constant's bits.
    Const(u64),
    /// A numeric instruction without immediates: its opcode, `0x45..=0xC4`,
    /// or `0x100 + n` for the saturating truncations `0xFC n`.
    Numeric(u16),
    RefNull,
    RefIsNull,
    RefFunc(u32),
    MemoryInit(u32),
    DataDrop(u32),
    MemoryCopy,
    MemoryFill,
    TableInit {
        elem: u32,
        table: u32,
    },
    ElemDrop(u32),
    TableCopy {
        destination: u32,
        source: u32,
    },
    TableGrow(u32),
    TableSize(u32),
    TableFill(u32),
}

/// A compiled function body.
#[derive(Debug)]
pub(crate) struct Body {
    pub(crate) ops: Vec<Op>,
    pub(crate) targets: Vec<Target>,
    /// Parameters and declared locals: the frame's slots below its operands.
    pub(crate) locals: u32,
    pub(crate) params: u32,
    pub(crate) results: u32,
    /// The deepest the operand stack goes above the locals.
    pub(crate) max_height: u32,
}

/// What compiling a module's bodies needs from the module.
pub(crate) struct Context<'a> {
    module: &'a Module,
    functions: Vec<u32>,
    globals: Vec<GlobalType>,
    tables: Vec<TableType>,
    memories: usize,
    data_count: Option<u32>,
    /// Functions `ref.func` may name: those in element segments, exports and
    /// globals.
    declared: Vec<bool>,
}

impl<'a> Context<'a> {
    pub(crate) fn new(module: &'a Module, data_count: Option<u32>) -> Context<'a> {
        let mut functions: Vec<u32> = module
            .imports
            .iter()
            .filter_map(|import| match import.desc {
                super::decode::ImportDesc::Func(ty) => Some(ty),
                _ => None,
            })
            .collect();
        functions.extend(module.funcs.iter().copied());
        let mut declared = vec![false; functions.len()];
        let mut declare = |expr: &super::decode::ConstExpr| {
            for op in expr {
                if let super::decode::ConstOp::RefFunc(index) = op
                    && let Some(slot) = declared.get_mut(*index as usize)
                {
                    *slot = true;
                }
            }
        };
        for elem in &module.elems {
            for item in &elem.items {
                declare(item);
            }
        }
        for global in &module.globals {
            declare(&global.init);
        }
        for export in &module.exports {
            if export.kind == ExternKind::Func
                && let Some(slot) = declared.get_mut(export.index as usize)
            {
                *slot = true;
            }
        }
        Context {
            module,
            functions,
            globals: module.global_types(),
            tables: module.table_types(),
            memories: module.memory_count(),
            data_count,
            declared,
        }
    }
}

/// A block being compiled.
struct Control {
    /// `true` for a loop, whose branches go to its start.
    is_loop: bool,
    start: u32,
    /// The operand height below the block's parameters.
    height: u32,
    params: u32,
    results: u32,
    /// Jumps to patch with the block's end: `(op index, kind)`.
    patches: Vec<(usize, Patch)>,
    /// A pending `if`'s conditional jump, to patch at `else` or `end`.
    else_jump: Option<usize>,
    /// Whether the rest of the block is unreachable.
    unreachable: bool,
}

#[derive(Clone, Copy)]
enum Patch {
    Op,
    Table(usize),
}

struct Compiler<'c, 'a> {
    context: &'c Context<'a>,
    ops: Vec<Op>,
    targets: Vec<Target>,
    controls: Vec<Control>,
    height: u32,
    max_height: u32,
    locals: Vec<ValType>,
}

const MAX_LOCALS: u64 = 50_000;

/// Compiles a function body.
pub(crate) fn compile(
    context: &Context<'_>,
    ty: &FuncType,
    reader: &mut Reader<'_>,
) -> Decoded<Body> {
    let mut locals = ty.params.clone();
    let mut total = locals.len() as u64;
    for _ in 0..reader.u32()? {
        let count = reader.u32()?;
        let local = reader.value_type()?;
        total += u64::from(count);
        if total > MAX_LOCALS {
            return reader.error("local count too large");
        }
        locals.extend(core::iter::repeat_n(local, count as usize));
    }
    let mut compiler = Compiler {
        context,
        ops: Vec::new(),
        targets: Vec::new(),
        controls: Vec::new(),
        height: 0,
        max_height: 0,
        locals,
    };
    compiler.controls.push(Control {
        is_loop: false,
        start: 0,
        height: 0,
        params: 0,
        results: ty.results.len() as u32,
        patches: Vec::new(),
        else_jump: None,
        unreachable: false,
    });
    compiler.body(reader)?;
    if !reader.done() {
        return reader.error("operators remaining after end of function");
    }
    Ok(Body {
        ops: compiler.ops,
        targets: compiler.targets,
        locals: compiler.locals.len() as u32,
        params: ty.params.len() as u32,
        results: ty.results.len() as u32,
        max_height: compiler.max_height,
    })
}

impl Compiler<'_, '_> {
    fn error<T>(&self, reader: &Reader<'_>, message: &str) -> Decoded<T> {
        Err(CompileError::new(message, reader.position))
    }

    fn unreachable(&self) -> bool {
        self.controls
            .last()
            .is_some_and(|control| control.unreachable)
    }

    fn emit(&mut self, op: Op) {
        if !self.unreachable() {
            self.ops.push(op);
        }
    }

    /// Applies an instruction's stack effect, checking the operands exist.
    fn effect(&mut self, reader: &Reader<'_>, pops: u32, pushes: u32) -> Decoded<()> {
        if self.unreachable() {
            return Ok(());
        }
        // Operands below the block's own, which include its parameters, are
        // not reachable from inside it.
        let base = self.controls.last().map_or(0, |control| control.height);
        if self.height < base + pops {
            return self.error(reader, "not enough arguments on the stack");
        }
        self.height = self.height - pops + pushes;
        self.max_height = self.max_height.max(self.height);
        Ok(())
    }

    fn block_type(&mut self, reader: &mut Reader<'_>) -> Decoded<(u32, u32)> {
        let start = reader.position;
        let byte = reader.bytes.get(reader.position).copied();
        match byte {
            Some(0x40) => {
                reader.position += 1;
                Ok((0, 0))
            }
            Some(byte) if ValType::from_byte(byte).is_some() => {
                reader.position += 1;
                Ok((0, 1))
            }
            _ => {
                let index = reader.s33()?;
                let ty = usize::try_from(index)
                    .ok()
                    .and_then(|index| self.context.module.types.get(index))
                    .ok_or_else(|| CompileError::new("block type index out of bounds", start))?;
                Ok((ty.params.len() as u32, ty.results.len() as u32))
            }
        }
    }

    fn local(&self, reader: &mut Reader<'_>) -> Decoded<u32> {
        let index = reader.u32()?;
        if index as usize >= self.locals.len() {
            return self.error(reader, &format!("invalid local index: {index}"));
        }
        Ok(index)
    }

    fn global(&self, reader: &mut Reader<'_>) -> Decoded<u32> {
        let index = reader.u32()?;
        if index as usize >= self.context.globals.len() {
            return self.error(reader, &format!("Invalid global index: {index}"));
        }
        Ok(index)
    }

    fn table(&self, reader: &mut Reader<'_>) -> Decoded<u32> {
        let index = reader.u32()?;
        if index as usize >= self.context.tables.len() {
            return self.error(reader, &format!("table index {index} out of bounds"));
        }
        Ok(index)
    }

    fn memory(&self, reader: &mut Reader<'_>) -> Decoded<()> {
        let index = reader.u32()?;
        if index as usize >= self.context.memories {
            return self.error(
                reader,
                &format!("memory index {index} exceeds number of declared memories"),
            );
        }
        Ok(())
    }

    fn data(&self, reader: &mut Reader<'_>) -> Decoded<u32> {
        let index = reader.u32()?;
        match self.context.data_count {
            None => self.error(reader, "data count section required"),
            Some(count) if index >= count => {
                self.error(reader, &format!("invalid data segment index: {index}"))
            }
            Some(_) => Ok(index),
        }
    }

    fn elem(&self, reader: &mut Reader<'_>) -> Decoded<u32> {
        let index = reader.u32()?;
        if index as usize >= self.context.module.elems.len() {
            return self.error(reader, &format!("invalid element segment index: {index}"));
        }
        Ok(index)
    }

    /// A memory instruction's offset, after its alignment, checked against
    /// the access's natural alignment.
    fn memarg(&self, reader: &mut Reader<'_>, natural: u32) -> Decoded<u32> {
        let start = reader.position;
        let align = reader.u32()?;
        let offset = reader.u32()?;
        if self.context.memories == 0 {
            return Err(CompileError::new(
                "memory instruction with no memory",
                start,
            ));
        }
        if align > natural {
            return Err(CompileError::new(
                format!(
                    "invalid alignment; expected maximum alignment is {natural}, actual alignment is {align}"
                ),
                start,
            ));
        }
        Ok(offset)
    }

    /// The branch target `depth` blocks out.
    fn target(&self, reader: &Reader<'_>, depth: u32) -> Decoded<(usize, Target)> {
        let count = self.controls.len();
        if depth as usize >= count {
            return self.error(reader, &format!("invalid branch depth: {depth}"));
        }
        let index = count - 1 - depth as usize;
        let control = &self.controls[index];
        let arity = if control.is_loop {
            control.params
        } else {
            control.results
        };
        if !self.unreachable() && self.height < self.controls[count - 1].height + arity {
            return self.error(reader, "expected more values on the stack for the branch");
        }
        Ok((
            index,
            Target {
                pc: control.start,
                height: control.height,
                arity,
            },
        ))
    }

    /// Records a jump to a block's end for patching, unless it targets a
    /// loop, whose start is known.
    fn patch(&mut self, index: usize, patch: Patch) {
        if !self.controls[index].is_loop {
            self.controls[index].patches.push((
                match patch {
                    Patch::Op => self.ops.len() - 1,
                    Patch::Table(_) => 0,
                },
                patch,
            ));
        }
    }

    fn end_unreachable(&mut self) {
        if let Some(control) = self.controls.last_mut() {
            control.unreachable = true;
        }
        if let Some(control) = self.controls.last() {
            self.height = control.height;
        }
    }

    fn call(&mut self, reader: &Reader<'_>, ty: &FuncType) -> Decoded<()> {
        self.effect(reader, ty.params.len() as u32, ty.results.len() as u32)
    }

    fn body(&mut self, reader: &mut Reader<'_>) -> Decoded<()> {
        loop {
            let opcode = reader.byte()?;
            match opcode {
                0x00 => {
                    self.emit(Op::Unreachable);
                    self.end_unreachable();
                }
                0x01 => {}
                0x02 | 0x03 => {
                    let (params, results) = self.block_type(reader)?;
                    let reachable = !self.unreachable();
                    if reachable
                        && self.height < self.controls.last().map_or(0, |c| c.height) + params
                    {
                        return self.error(reader, "not enough arguments on the stack for block");
                    }
                    let height = if reachable {
                        self.height - params
                    } else {
                        self.height
                    };
                    self.controls.push(Control {
                        is_loop: opcode == 0x03,
                        start: self.ops.len() as u32,
                        height,
                        params,
                        results,
                        patches: Vec::new(),
                        else_jump: None,
                        unreachable: !reachable,
                    });
                    if !reachable {
                        self.height = height;
                    }
                }
                0x04 => {
                    let (params, results) = self.block_type(reader)?;
                    self.effect(reader, 1, 0)?;
                    let reachable = !self.unreachable();
                    if reachable
                        && self.height < self.controls.last().map_or(0, |c| c.height) + params
                    {
                        return self.error(reader, "not enough arguments on the stack for if");
                    }
                    self.emit(Op::JumpIfZero(0));
                    let else_jump = reachable.then(|| self.ops.len() - 1);
                    let height = if reachable {
                        self.height - params
                    } else {
                        self.height
                    };
                    self.controls.push(Control {
                        is_loop: false,
                        start: 0,
                        height,
                        params,
                        results,
                        patches: Vec::new(),
                        else_jump,
                        unreachable: !reachable,
                    });
                }
                0x05 => {
                    let Some(control) = self.controls.last() else {
                        return self.error(reader, "else does not match an if");
                    };
                    if self.controls.len() == 1 {
                        return self.error(reader, "else does not match an if");
                    }
                    let entered = control.else_jump.is_some();
                    let outer_unreachable = self
                        .controls
                        .get(self.controls.len() - 2)
                        .is_some_and(|outer| outer.unreachable);
                    if !self.unreachable() && self.height != control.height + control.results {
                        return self.error(reader, "type error in if true branch");
                    }
                    // The true branch jumps over the false one.
                    self.emit(Op::Jump(0));
                    if !self.unreachable() {
                        let index = self.controls.len() - 1;
                        self.patch(index, Patch::Op);
                    }
                    let here = self.ops.len() as u32;
                    let control = self.controls.last_mut().expect("an if");
                    if let Some(jump) = control.else_jump.take() {
                        self.ops[jump] = Op::JumpIfZero(here);
                    }
                    control.unreachable = !entered || outer_unreachable;
                    self.height = control.height + control.params;
                    if control.unreachable {
                        self.height = control.height;
                    }
                }
                0x0B => {
                    let control = self.controls.last().expect("a block");
                    if !control.unreachable && self.height != control.height + control.results {
                        return self.error(
                            reader,
                            "expected a different number of values at the end of the block",
                        );
                    }
                    let control = self.controls.pop().expect("a block");
                    let end = self.ops.len() as u32;
                    if let Some(jump) = control.else_jump {
                        // An `if` without `else`: its parameters are its
                        // results.
                        if control.params != control.results {
                            return self.error(
                                reader,
                                "if without else must leave its parameters as results",
                            );
                        }
                        self.ops[jump] = Op::JumpIfZero(end);
                    }
                    for (index, patch) in control.patches {
                        match patch {
                            Patch::Op => match &mut self.ops[index] {
                                Op::Jump(pc) | Op::JumpIfZero(pc) => *pc = end,
                                Op::Br(target) | Op::BrIf(target) => target.pc = end,
                                _ => unreachable!("patched instructions jump"),
                            },
                            Patch::Table(slot) => self.targets[slot].pc = end,
                        }
                    }
                    self.height = control.height + control.results;
                    if self.controls.is_empty() {
                        self.ops.push(Op::Return);
                        return Ok(());
                    }
                    if self.unreachable() {
                        self.height = self.controls.last().map_or(0, |c| c.height);
                    }
                }
                0x0C => {
                    let depth = reader.u32()?;
                    let (index, target) = self.target(reader, depth)?;
                    if !self.unreachable() {
                        if self.height == target.height + target.arity {
                            self.emit(Op::Jump(target.pc));
                        } else {
                            self.emit(Op::Br(target));
                        }
                        self.patch(index, Patch::Op);
                    }
                    self.end_unreachable();
                }
                0x0D => {
                    let depth = reader.u32()?;
                    self.effect(reader, 1, 0)?;
                    let (index, target) = self.target(reader, depth)?;
                    if !self.unreachable() {
                        self.emit(Op::BrIf(target));
                        self.patch(index, Patch::Op);
                    }
                }
                0x0E => {
                    let count = reader.u32()?;
                    if count > 1_000_000 {
                        return self.error(reader, "br_table too large");
                    }
                    self.effect(reader, 1, 0)?;
                    let start = self.targets.len() as u32;
                    let mut arity = None;
                    for _ in 0..=count {
                        let depth = reader.u32()?;
                        let (index, target) = self.target(reader, depth)?;
                        if arity.is_some_and(|arity| arity != target.arity) {
                            return self.error(reader, "type error in br_table");
                        }
                        arity = Some(target.arity);
                        if !self.unreachable() {
                            self.targets.push(target);
                            let slot = self.targets.len() - 1;
                            self.patch_table(index, slot);
                        }
                    }
                    self.emit(Op::BrTable {
                        start,
                        count: count + 1,
                    });
                    self.end_unreachable();
                }
                0x0F => {
                    let results = self.controls[0].results;
                    self.effect(reader, results, results)?;
                    self.emit(Op::Return);
                    self.end_unreachable();
                }
                0x10 => {
                    let index = reader.u32()?;
                    let Some(&ty) = self.context.functions.get(index as usize) else {
                        return self
                            .error(reader, &format!("function index #{index} is out of bounds"));
                    };
                    let ty = self.context.module.types[ty as usize].clone();
                    self.call(reader, &ty)?;
                    self.emit(Op::Call(index));
                }
                0x11 => {
                    let ty = reader.u32()?;
                    let table = reader.u32()?;
                    if table as usize >= self.context.tables.len() {
                        return self.error(reader, &format!("table index {table} out of bounds"));
                    }
                    if self.context.tables[table as usize].element != ValType::FuncRef {
                        return self.error(reader, "call_indirect: table must be of type funcref");
                    }
                    let Some(signature) = self.context.module.types.get(ty as usize).cloned()
                    else {
                        return self.error(reader, &format!("invalid signature index: {ty}"));
                    };
                    self.effect(reader, 1, 0)?;
                    self.call(reader, &signature)?;
                    self.emit(Op::CallIndirect { ty, table });
                }
                0x1A => {
                    self.effect(reader, 1, 0)?;
                    self.emit(Op::Drop);
                }
                0x1B => {
                    self.effect(reader, 3, 1)?;
                    self.emit(Op::Select);
                }
                0x1C => {
                    let count = reader.u32()?;
                    if count != 1 {
                        return self.error(reader, "invalid number of types for select");
                    }
                    reader.value_type()?;
                    self.effect(reader, 3, 1)?;
                    self.emit(Op::Select);
                }
                0x20 => {
                    let index = self.local(reader)?;
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::LocalGet(index));
                }
                0x21 => {
                    let index = self.local(reader)?;
                    self.effect(reader, 1, 0)?;
                    self.emit(Op::LocalSet(index));
                }
                0x22 => {
                    let index = self.local(reader)?;
                    self.effect(reader, 1, 1)?;
                    self.emit(Op::LocalTee(index));
                }
                0x23 => {
                    let index = self.global(reader)?;
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::GlobalGet(index));
                }
                0x24 => {
                    let index = self.global(reader)?;
                    if !self.context.globals[index as usize].mutable {
                        return self.error(
                            reader,
                            &format!("immutable global #{index} cannot be assigned"),
                        );
                    }
                    self.effect(reader, 1, 0)?;
                    self.emit(Op::GlobalSet(index));
                }
                0x25 => {
                    let index = self.table(reader)?;
                    self.effect(reader, 1, 1)?;
                    self.emit(Op::TableGet(index));
                }
                0x26 => {
                    let index = self.table(reader)?;
                    self.effect(reader, 2, 0)?;
                    self.emit(Op::TableSet(index));
                }
                0x28..=0x35 => {
                    let natural = match opcode {
                        0x28 | 0x2A => 2,
                        0x29 | 0x2B => 3,
                        0x2C | 0x2D | 0x30 | 0x31 => 0,
                        0x2E | 0x2F | 0x32 | 0x33 => 1,
                        _ => 2,
                    };
                    let offset = self.memarg(reader, natural)?;
                    self.effect(reader, 1, 1)?;
                    self.emit(match opcode {
                        0x28 => Op::I32Load(offset),
                        0x29 => Op::I64Load(offset),
                        0x2A => Op::F32Load(offset),
                        0x2B => Op::F64Load(offset),
                        0x2C => Op::I32Load8S(offset),
                        0x2D => Op::I32Load8U(offset),
                        0x2E => Op::I32Load16S(offset),
                        0x2F => Op::I32Load16U(offset),
                        0x30 => Op::I64Load8S(offset),
                        0x31 => Op::I64Load8U(offset),
                        0x32 => Op::I64Load16S(offset),
                        0x33 => Op::I64Load16U(offset),
                        0x34 => Op::I64Load32S(offset),
                        _ => Op::I64Load32U(offset),
                    });
                }
                0x36..=0x3E => {
                    let natural = match opcode {
                        0x36 | 0x38 | 0x3E => 2,
                        0x37 | 0x39 => 3,
                        0x3A | 0x3C => 0,
                        _ => 1,
                    };
                    let offset = self.memarg(reader, natural)?;
                    self.effect(reader, 2, 0)?;
                    self.emit(match opcode {
                        0x36 => Op::I32Store(offset),
                        0x37 => Op::I64Store(offset),
                        0x38 => Op::F32Store(offset),
                        0x39 => Op::F64Store(offset),
                        0x3A => Op::I32Store8(offset),
                        0x3B => Op::I32Store16(offset),
                        0x3C => Op::I64Store8(offset),
                        0x3D => Op::I64Store16(offset),
                        _ => Op::I64Store32(offset),
                    });
                }
                0x3F => {
                    self.memory(reader)?;
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::MemorySize);
                }
                0x40 => {
                    self.memory(reader)?;
                    self.effect(reader, 1, 1)?;
                    self.emit(Op::MemoryGrow);
                }
                0x41 => {
                    let value = reader.s32()?;
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::Const(u64::from(value as u32)));
                }
                0x42 => {
                    let value = reader.s64()?;
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::Const(value as u64));
                }
                0x43 => {
                    let bits = reader.fixed32()?;
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::Const(u64::from(bits)));
                }
                0x44 => {
                    let bits = reader.fixed64()?;
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::Const(bits));
                }
                0x45..=0xC4 => {
                    let (pops, pushes) = numeric_effect(opcode);
                    self.effect(reader, pops, pushes)?;
                    self.emit(Op::Numeric(u16::from(opcode)));
                }
                0xD0 => {
                    let start = reader.position;
                    match reader.value_type()? {
                        ValType::FuncRef | ValType::ExternRef => {}
                        _ => return Err(CompileError::new("invalid reference type", start)),
                    }
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::RefNull);
                }
                0xD1 => {
                    self.effect(reader, 1, 1)?;
                    self.emit(Op::RefIsNull);
                }
                0xD2 => {
                    let index = reader.u32()?;
                    if index as usize >= self.context.functions.len() {
                        return self
                            .error(reader, &format!("function index #{index} is out of bounds"));
                    }
                    if !self.context.declared[index as usize] {
                        return self.error(
                            reader,
                            &format!("undeclared reference to function #{index}"),
                        );
                    }
                    self.effect(reader, 0, 1)?;
                    self.emit(Op::RefFunc(index));
                }
                0xFC => {
                    let sub = reader.u32()?;
                    match sub {
                        0..=7 => {
                            self.effect(reader, 1, 1)?;
                            self.emit(Op::Numeric(0x100 + sub as u16));
                        }
                        8 => {
                            let segment = self.data(reader)?;
                            self.memory(reader)?;
                            self.effect(reader, 3, 0)?;
                            self.emit(Op::MemoryInit(segment));
                        }
                        9 => {
                            let segment = self.data(reader)?;
                            self.emit(Op::DataDrop(segment));
                        }
                        10 => {
                            self.memory(reader)?;
                            self.memory(reader)?;
                            self.effect(reader, 3, 0)?;
                            self.emit(Op::MemoryCopy);
                        }
                        11 => {
                            self.memory(reader)?;
                            self.effect(reader, 3, 0)?;
                            self.emit(Op::MemoryFill);
                        }
                        12 => {
                            let elem = self.elem(reader)?;
                            let table = self.table(reader)?;
                            self.effect(reader, 3, 0)?;
                            self.emit(Op::TableInit { elem, table });
                        }
                        13 => {
                            let elem = self.elem(reader)?;
                            self.emit(Op::ElemDrop(elem));
                        }
                        14 => {
                            let destination = self.table(reader)?;
                            let source = self.table(reader)?;
                            self.effect(reader, 3, 0)?;
                            self.emit(Op::TableCopy {
                                destination,
                                source,
                            });
                        }
                        15 => {
                            let table = self.table(reader)?;
                            self.effect(reader, 2, 1)?;
                            self.emit(Op::TableGrow(table));
                        }
                        16 => {
                            let table = self.table(reader)?;
                            self.effect(reader, 0, 1)?;
                            self.emit(Op::TableSize(table));
                        }
                        17 => {
                            let table = self.table(reader)?;
                            self.effect(reader, 3, 0)?;
                            self.emit(Op::TableFill(table));
                        }
                        _ => {
                            return self
                                .error(reader, &format!("invalid numeric opcode: 0xfc{sub:02x}"));
                        }
                    }
                }
                0xFD => return self.error(reader, "Wasm SIMD unsupported"),
                0xFE => return self.error(reader, "invalid atomic opcode"),
                _ => return self.error(reader, &format!("invalid opcode 0x{opcode:x}")),
            }
        }
    }

    fn patch_table(&mut self, index: usize, slot: usize) {
        if !self.controls[index].is_loop {
            self.controls[index].patches.push((0, Patch::Table(slot)));
        }
    }
}

/// The operands a numeric instruction pops and pushes.
fn numeric_effect(opcode: u8) -> (u32, u32) {
    match opcode {
        0x45 | 0x50 => (1, 1),
        0x46..=0x4F | 0x51..=0x66 => (2, 1),
        0x67..=0x69 | 0x79..=0x7B | 0x8B..=0x91 | 0x99..=0x9F => (1, 1),
        0x6A..=0x78 | 0x7C..=0x8A | 0x92..=0x98 | 0xA0..=0xA6 => (2, 1),
        _ => (1, 1),
    }
}
