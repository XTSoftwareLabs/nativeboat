//! The WebAssembly binary format: a module's sections, decoded into the
//! types, imports, definitions and exports instantiation needs, with each
//! function's code compiled by [`super::compile`].

#[allow(unused_imports)]
use crate::prelude::*;

use super::compile::{self, Body};
use super::{FuncType, GlobalType, Limits, TableType, ValType};
use alloc::rc::Rc;
use core::ops::Range;

/// A decoding or validation failure, with the byte offset it was found at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompileError {
    pub(crate) message: String,
    pub(crate) offset: usize,
}

impl CompileError {
    pub(crate) fn new(message: impl Into<String>, offset: usize) -> CompileError {
        CompileError {
            message: message.into(),
            offset,
        }
    }

    /// The message as V8 words it, with the offset.
    pub(crate) fn describe(&self) -> String {
        format!("{} @+{}", self.message, self.offset)
    }
}

pub(crate) type Decoded<T> = Result<T, CompileError>;

/// What an import or export names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternKind {
    Func,
    Table,
    Memory,
    Global,
    Tag,
}

impl ExternKind {
    /// The kind as the JavaScript API names it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            ExternKind::Func => "function",
            ExternKind::Table => "table",
            ExternKind::Memory => "memory",
            ExternKind::Global => "global",
            ExternKind::Tag => "tag",
        }
    }
}

/// An import's description.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImportDesc {
    Func(u32),
    Table(TableType),
    Memory(Limits),
    Global(GlobalType),
}

impl ImportDesc {
    pub(crate) fn kind(&self) -> ExternKind {
        match self {
            ImportDesc::Func(_) => ExternKind::Func,
            ImportDesc::Table(_) => ExternKind::Table,
            ImportDesc::Memory(_) => ExternKind::Memory,
            ImportDesc::Global(_) => ExternKind::Global,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Import {
    pub(crate) module: String,
    pub(crate) name: String,
    pub(crate) desc: ImportDesc,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Export {
    pub(crate) name: String,
    pub(crate) kind: ExternKind,
    pub(crate) index: u32,
}

/// A constant expression: global initializers and segment offsets, with the
/// extended-constant arithmetic.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConstOp {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    GlobalGet(u32),
    RefNull(ValType),
    RefFunc(u32),
    I32Add,
    I32Sub,
    I32Mul,
    I64Add,
    I64Sub,
    I64Mul,
}

pub(crate) type ConstExpr = Vec<ConstOp>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Global {
    pub(crate) ty: GlobalType,
    pub(crate) init: ConstExpr,
}

/// Where an element or data segment goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SegmentMode {
    Passive,
    Active { index: u32, offset: ConstExpr },
    Declarative,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Elem {
    pub(crate) ty: ValType,
    pub(crate) mode: SegmentMode,
    pub(crate) items: Vec<ConstExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Data {
    pub(crate) mode: SegmentMode,
    pub(crate) bytes: Range<usize>,
}

/// A decoded and validated module.
#[derive(Debug)]
pub(crate) struct Module {
    pub(crate) bytes: Vec<u8>,
    pub(crate) types: Vec<FuncType>,
    pub(crate) imports: Vec<Import>,
    /// The type of each function the module defines.
    pub(crate) funcs: Vec<u32>,
    pub(crate) tables: Vec<TableType>,
    pub(crate) memories: Vec<Limits>,
    pub(crate) globals: Vec<Global>,
    pub(crate) exports: Vec<Export>,
    pub(crate) start: Option<u32>,
    pub(crate) elems: Vec<Elem>,
    pub(crate) datas: Vec<Data>,
    /// The compiled body of each defined function.
    pub(crate) bodies: Vec<Rc<Body>>,
    pub(crate) customs: Vec<(String, Range<usize>)>,
}

impl Module {
    /// The number of imported functions, which come first in the function
    /// index space.
    pub(crate) fn imported(&self, kind: ExternKind) -> usize {
        self.imports
            .iter()
            .filter(|import| import.desc.kind() == kind)
            .count()
    }

    /// The type of a function in the index space.
    pub(crate) fn func_type(&self, index: u32) -> Option<&FuncType> {
        let mut index = index as usize;
        for import in &self.imports {
            if let ImportDesc::Func(ty) = import.desc {
                if index == 0 {
                    return self.types.get(ty as usize);
                }
                index -= 1;
            }
        }
        self.types.get(*self.funcs.get(index)? as usize)
    }

    /// The type of each global in the index space.
    pub(crate) fn global_types(&self) -> Vec<GlobalType> {
        let mut types: Vec<GlobalType> = self
            .imports
            .iter()
            .filter_map(|import| match import.desc {
                ImportDesc::Global(ty) => Some(ty),
                _ => None,
            })
            .collect();
        types.extend(self.globals.iter().map(|global| global.ty));
        types
    }

    /// The element type of each table in the index space.
    pub(crate) fn table_types(&self) -> Vec<TableType> {
        let mut types: Vec<TableType> = self
            .imports
            .iter()
            .filter_map(|import| match import.desc {
                ImportDesc::Table(ty) => Some(ty),
                _ => None,
            })
            .collect();
        types.extend(self.tables.iter().copied());
        types
    }

    /// The number of memories in the index space.
    pub(crate) fn memory_count(&self) -> usize {
        self.imported(ExternKind::Memory) + self.memories.len()
    }
}

/// A cursor over the module's bytes.
pub(crate) struct Reader<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) position: usize,
    pub(crate) end: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8], position: usize, end: usize) -> Reader<'a> {
        Reader {
            bytes,
            position,
            end,
        }
    }

    pub(crate) fn error<T>(&self, message: &str) -> Decoded<T> {
        Err(CompileError::new(message, self.position))
    }

    pub(crate) fn done(&self) -> bool {
        self.position >= self.end
    }

    pub(crate) fn byte(&mut self) -> Decoded<u8> {
        if self.position >= self.end {
            return self.error("unexpected end of section or function");
        }
        let byte = self.bytes[self.position];
        self.position += 1;
        Ok(byte)
    }

    pub(crate) fn take(&mut self, length: usize) -> Decoded<Range<usize>> {
        if self.end - self.position < length {
            return self.error("unexpected end of section or function");
        }
        let start = self.position;
        self.position += length;
        Ok(start..self.position)
    }

    fn unsigned(&mut self, bits: u32) -> Decoded<u64> {
        let mut result = 0u64;
        let mut shift = 0;
        loop {
            let byte = self.byte()?;
            result |= u64::from(byte & 0x7F) << shift;
            if shift + 7 >= bits {
                if byte & 0x80 != 0 {
                    return self.error("length overflow while decoding integer");
                }
                if (byte & 0x7F) >> (bits - shift) != 0 {
                    return self.error("extra bits in varint");
                }
                return Ok(result);
            }
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    fn signed(&mut self, bits: u32) -> Decoded<i64> {
        let mut result = 0i64;
        let mut shift = 0;
        loop {
            let byte = self.byte()?;
            result |= i64::from(byte & 0x7F) << shift;
            let last = shift + 7 >= bits;
            if last {
                if byte & 0x80 != 0 {
                    return self.error("length overflow while decoding integer");
                }
                // The bits past the width must extend its sign.
                let sign_and_unused = ((byte << 1) as i8) >> (bits - shift);
                if sign_and_unused != 0 && sign_and_unused != -1 {
                    return self.error("extra bits in varint");
                }
            }
            if last || byte & 0x80 == 0 {
                if shift + 7 < 64 && byte & 0x40 != 0 {
                    result |= -1i64 << (shift + 7);
                }
                return Ok(result);
            }
            shift += 7;
        }
    }

    pub(crate) fn u32(&mut self) -> Decoded<u32> {
        self.unsigned(32).map(|value| value as u32)
    }

    pub(crate) fn s32(&mut self) -> Decoded<i32> {
        self.signed(32).map(|value| value as i32)
    }

    pub(crate) fn s33(&mut self) -> Decoded<i64> {
        self.signed(33)
    }

    pub(crate) fn s64(&mut self) -> Decoded<i64> {
        self.signed(64)
    }

    pub(crate) fn fixed32(&mut self) -> Decoded<u32> {
        let range = self.take(4)?;
        Ok(u32::from_le_bytes(
            self.bytes[range].try_into().expect("four bytes"),
        ))
    }

    pub(crate) fn fixed64(&mut self) -> Decoded<u64> {
        let range = self.take(8)?;
        Ok(u64::from_le_bytes(
            self.bytes[range].try_into().expect("eight bytes"),
        ))
    }

    fn name(&mut self) -> Decoded<String> {
        let length = self.u32()? as usize;
        let start = self.position;
        let range = self.take(length)?;
        match core::str::from_utf8(&self.bytes[range]) {
            Ok(text) => Ok(text.to_owned()),
            Err(_) => Err(CompileError::new("invalid UTF-8 string", start)),
        }
    }

    pub(crate) fn value_type(&mut self) -> Decoded<ValType> {
        let start = self.position;
        ValType::from_byte(self.byte()?)
            .ok_or_else(|| CompileError::new("invalid value type", start))
    }

    fn reference_type(&mut self) -> Decoded<ValType> {
        let start = self.position;
        match self.value_type()? {
            ty @ (ValType::FuncRef | ValType::ExternRef) => Ok(ty),
            _ => Err(CompileError::new("invalid reference type", start)),
        }
    }

    fn limits(&mut self, maximum: u32) -> Decoded<Limits> {
        let start = self.position;
        let flags = self.byte()?;
        if flags > 1 {
            return Err(CompileError::new(
                format!("invalid limits flags 0x{flags:x}"),
                start,
            ));
        }
        let min = self.u32()?;
        let max = if flags & 1 == 1 {
            Some(self.u32()?)
        } else {
            None
        };
        if min > maximum || max.is_some_and(|max| max > maximum) {
            return Err(CompileError::new("size is out of bounds", start));
        }
        if max.is_some_and(|max| max < min) {
            return Err(CompileError::new(
                "maximum size is smaller than the initial size",
                start,
            ));
        }
        Ok(Limits { min, max })
    }

    fn table_type(&mut self) -> Decoded<TableType> {
        let element = self.reference_type()?;
        let limits = self.limits(u32::MAX)?;
        Ok(TableType { element, limits })
    }

    fn global_type(&mut self) -> Decoded<GlobalType> {
        let ty = self.value_type()?;
        let start = self.position;
        let mutable = match self.byte()? {
            0 => false,
            1 => true,
            _ => return Err(CompileError::new("invalid global mutability", start)),
        };
        Ok(GlobalType { ty, mutable })
    }

    fn const_expr(&mut self) -> Decoded<ConstExpr> {
        let mut ops = Vec::new();
        loop {
            let start = self.position;
            let op = match self.byte()? {
                0x0B => return Ok(ops),
                0x41 => ConstOp::I32(self.s32()?),
                0x42 => ConstOp::I64(self.s64()?),
                0x43 => ConstOp::F32(self.fixed32()?),
                0x44 => ConstOp::F64(self.fixed64()?),
                0x23 => ConstOp::GlobalGet(self.u32()?),
                0xD0 => ConstOp::RefNull(self.reference_type()?),
                0xD2 => ConstOp::RefFunc(self.u32()?),
                0x6A => ConstOp::I32Add,
                0x6B => ConstOp::I32Sub,
                0x6C => ConstOp::I32Mul,
                0x7C => ConstOp::I64Add,
                0x7D => ConstOp::I64Sub,
                0x7E => ConstOp::I64Mul,
                _ => {
                    return Err(CompileError::new(
                        "opcode is not allowed in constant expressions",
                        start,
                    ));
                }
            };
            ops.push(op);
        }
    }
}

const PAGE_LIMIT: u32 = 65536;

/// Decodes and validates a module.
pub(crate) fn decode(bytes: Vec<u8>) -> Decoded<Module> {
    if bytes.len() < 4 || bytes[..4] != *b"\0asm" {
        let found: Vec<String> = bytes
            .iter()
            .take(4)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        return Err(CompileError::new(
            format!("expected magic word 00 61 73 6d, found {}", found.join(" ")),
            0,
        ));
    }
    if bytes.len() < 8 || bytes[4..8] != [1, 0, 0, 0] {
        let found: Vec<String> = bytes
            .iter()
            .skip(4)
            .take(4)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        return Err(CompileError::new(
            format!("expected version 01 00 00 00, found {}", found.join(" ")),
            4,
        ));
    }
    let mut module = Module {
        bytes: Vec::new(),
        types: Vec::new(),
        imports: Vec::new(),
        funcs: Vec::new(),
        tables: Vec::new(),
        memories: Vec::new(),
        globals: Vec::new(),
        exports: Vec::new(),
        start: None,
        elems: Vec::new(),
        datas: Vec::new(),
        bodies: Vec::new(),
        customs: Vec::new(),
    };
    let mut codes: Vec<(usize, usize)> = Vec::new();
    let mut data_count = None;
    let mut last = 0u8;
    let mut reader = Reader::new(&bytes, 8, bytes.len());
    while !reader.done() {
        let section_start = reader.position;
        let id = reader.byte()?;
        let size = reader.u32()? as usize;
        let range = reader.take(size)?;
        let mut section = Reader::new(&bytes, range.start, range.end);
        if id != 0 {
            // Sections come in order, data count before code.
            let order = |id: u8| if id == 12 { 19 } else { id * 2 };
            if order(id) <= last {
                return Err(CompileError::new(
                    format!("unexpected section <{}>", section_name(id)),
                    section_start,
                ));
            }
            last = order(id);
        }
        match id {
            0 => {
                let name = section.name()?;
                module.customs.push((name, section.position..section.end));
                section.position = section.end;
            }
            1 => {
                for _ in 0..section.u32()? {
                    let start = section.position;
                    if section.byte()? != 0x60 {
                        return Err(CompileError::new("invalid function type form", start));
                    }
                    let mut params = Vec::new();
                    for _ in 0..section.u32()? {
                        params.push(section.value_type()?);
                    }
                    let mut results = Vec::new();
                    for _ in 0..section.u32()? {
                        results.push(section.value_type()?);
                    }
                    module.types.push(FuncType { params, results });
                }
            }
            2 => {
                for _ in 0..section.u32()? {
                    let module_name = section.name()?;
                    let name = section.name()?;
                    let start = section.position;
                    let desc = match section.byte()? {
                        0 => {
                            let ty = section.u32()?;
                            if ty as usize >= module.types.len() {
                                return Err(CompileError::new(
                                    "signature index out of bounds",
                                    start,
                                ));
                            }
                            ImportDesc::Func(ty)
                        }
                        1 => ImportDesc::Table(section.table_type()?),
                        2 => ImportDesc::Memory(section.limits(PAGE_LIMIT)?),
                        3 => ImportDesc::Global(section.global_type()?),
                        kind => {
                            return Err(CompileError::new(
                                format!("unknown import kind 0x{kind:02x}"),
                                start,
                            ));
                        }
                    };
                    module.imports.push(Import {
                        module: module_name,
                        name,
                        desc,
                    });
                }
            }
            3 => {
                for _ in 0..section.u32()? {
                    let start = section.position;
                    let ty = section.u32()?;
                    if ty as usize >= module.types.len() {
                        return Err(CompileError::new("signature index out of bounds", start));
                    }
                    module.funcs.push(ty);
                }
            }
            4 => {
                for _ in 0..section.u32()? {
                    module.tables.push(section.table_type()?);
                }
            }
            5 => {
                for _ in 0..section.u32()? {
                    module.memories.push(section.limits(PAGE_LIMIT)?);
                }
            }
            6 => {
                for _ in 0..section.u32()? {
                    let ty = section.global_type()?;
                    let init = section.const_expr()?;
                    module.globals.push(Global { ty, init });
                }
            }
            7 => {
                for _ in 0..section.u32()? {
                    let start = section.position;
                    let name = section.name()?;
                    let kind = match section.byte()? {
                        0 => ExternKind::Func,
                        1 => ExternKind::Table,
                        2 => ExternKind::Memory,
                        3 => ExternKind::Global,
                        4 => ExternKind::Tag,
                        kind => {
                            return Err(CompileError::new(
                                format!("invalid export kind 0x{kind:02x}"),
                                start,
                            ));
                        }
                    };
                    let index = section.u32()?;
                    if module.exports.iter().any(|export| export.name == name) {
                        return Err(CompileError::new(
                            format!("Duplicate export name '{name}'"),
                            start,
                        ));
                    }
                    module.exports.push(Export { name, kind, index });
                }
            }
            8 => module.start = Some(section.u32()?),
            9 => {
                for _ in 0..section.u32()? {
                    module.elems.push(elem(&mut section)?);
                }
            }
            12 => data_count = Some(section.u32()?),
            10 => {
                let count = section.u32()?;
                if count as usize != module.funcs.len() {
                    return Err(CompileError::new(
                        format!(
                            "function body count {count} mismatch ({} expected)",
                            module.funcs.len()
                        ),
                        section_start,
                    ));
                }
                for _ in 0..count {
                    let size = section.u32()? as usize;
                    let body = section.take(size)?;
                    codes.push((body.start, body.end));
                }
            }
            11 => {
                let count = section.u32()?;
                if data_count.is_some_and(|expected| expected != count) {
                    return Err(CompileError::new(
                        "data segments count mismatch",
                        section_start,
                    ));
                }
                for _ in 0..count {
                    let start = section.position;
                    let mode = match section.u32()? {
                        0 => SegmentMode::Active {
                            index: 0,
                            offset: section.const_expr()?,
                        },
                        1 => SegmentMode::Passive,
                        2 => SegmentMode::Active {
                            index: section.u32()?,
                            offset: section.const_expr()?,
                        },
                        _ => return Err(CompileError::new("invalid data segment flags", start)),
                    };
                    let length = section.u32()? as usize;
                    let bytes = section.take(length)?;
                    module.datas.push(Data { mode, bytes });
                }
            }
            _ => {
                return Err(CompileError::new(
                    format!("unknown section code #0x{id:x}"),
                    section_start,
                ));
            }
        }
        if !section.done() {
            return Err(CompileError::new(
                format!("section was shorter than expected size ({size} bytes expected)"),
                section.position,
            ));
        }
    }
    if codes.len() != module.funcs.len() {
        return Err(CompileError::new(
            format!(
                "function count is {}, but code section is absent",
                module.funcs.len()
            ),
            bytes.len(),
        ));
    }
    validate_indexes(&module, data_count)?;
    let context = compile::Context::new(&module, data_count);
    let mut bodies = Vec::with_capacity(codes.len());
    for (index, &(start, end)) in codes.iter().enumerate() {
        let ty = &module.types[module.funcs[index] as usize];
        let mut reader = Reader::new(&bytes, start, end);
        bodies.push(Rc::new(compile::compile(&context, ty, &mut reader)?));
    }
    drop(context);
    module.bodies = bodies;
    module.bytes = bytes;
    Ok(module)
}

fn elem(section: &mut Reader<'_>) -> Decoded<Elem> {
    let start = section.position;
    let flags = section.u32()?;
    if flags > 7 {
        return Err(CompileError::new("invalid element segment flags", start));
    }
    let mode = if flags & 1 == 0 {
        let index = if flags & 2 != 0 { section.u32()? } else { 0 };
        SegmentMode::Active {
            index,
            offset: section.const_expr()?,
        }
    } else if flags & 2 == 0 {
        SegmentMode::Passive
    } else {
        SegmentMode::Declarative
    };
    let expressions = flags & 4 != 0;
    let ty = if flags & 3 == 0 {
        ValType::FuncRef
    } else if expressions {
        section.reference_type()?
    } else {
        let kind_start = section.position;
        if section.byte()? != 0 {
            return Err(CompileError::new("invalid element kind", kind_start));
        }
        ValType::FuncRef
    };
    let mut items = Vec::new();
    for _ in 0..section.u32()? {
        if expressions {
            items.push(section.const_expr()?);
        } else {
            items.push(vec![ConstOp::RefFunc(section.u32()?)]);
        }
    }
    Ok(Elem { ty, mode, items })
}

fn section_name(id: u8) -> &'static str {
    match id {
        1 => "Type",
        2 => "Import",
        3 => "Function",
        4 => "Table",
        5 => "Memory",
        6 => "Global",
        7 => "Export",
        8 => "Start",
        9 => "Element",
        10 => "Code",
        11 => "Data",
        12 => "DataCount",
        _ => "Unknown",
    }
}

/// Checks the indexes exports, the start function and segments name.
fn validate_indexes(module: &Module, data_count: Option<u32>) -> Decoded<()> {
    let functions = module.imported(ExternKind::Func) + module.funcs.len();
    let tables = module.table_types().len();
    let memories = module.memory_count();
    let globals = module.global_types().len();
    if memories > 1 {
        return Err(CompileError::new("At most one memory is supported", 0));
    }
    for export in &module.exports {
        let count = match export.kind {
            ExternKind::Func => functions,
            ExternKind::Table => tables,
            ExternKind::Memory => memories,
            ExternKind::Global => globals,
            ExternKind::Tag => 0,
        };
        if export.index as usize >= count {
            return Err(CompileError::new(
                format!("invalid {} index: {}", export.kind.name(), export.index),
                0,
            ));
        }
    }
    if let Some(start) = module.start {
        let ty = module
            .func_type(start)
            .ok_or_else(|| CompileError::new(format!("function index {start} out of bounds"), 0))?;
        if !ty.params.is_empty() || !ty.results.is_empty() {
            return Err(CompileError::new(
                "invalid start function: non-zero parameter or return count",
                0,
            ));
        }
    }
    let check_const = |expr: &ConstExpr| -> Decoded<()> {
        for op in expr {
            match op {
                ConstOp::GlobalGet(index) if *index as usize >= globals => {
                    return Err(CompileError::new(
                        format!("Invalid global index: {index}"),
                        0,
                    ));
                }
                ConstOp::RefFunc(index) if *index as usize >= functions => {
                    return Err(CompileError::new(
                        format!("function index #{index} is out of bounds"),
                        0,
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    };
    for global in &module.globals {
        check_const(&global.init)?;
    }
    for elem in &module.elems {
        if let SegmentMode::Active { index, offset } = &elem.mode {
            if *index as usize >= tables {
                return Err(CompileError::new(
                    format!("out of bounds table index {index}"),
                    0,
                ));
            }
            check_const(offset)?;
        }
        for item in &elem.items {
            check_const(item)?;
        }
    }
    for data in &module.datas {
        if let SegmentMode::Active { index, offset } = &data.mode {
            if *index as usize >= memories {
                return Err(CompileError::new(
                    format!("invalid memory index {index}"),
                    0,
                ));
            }
            check_const(offset)?;
        }
    }
    let _ = data_count;
    Ok(())
}
