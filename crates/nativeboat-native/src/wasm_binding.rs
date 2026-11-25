//! `internalBinding('wasm')`: the native half of the WebAssembly JavaScript
//! API (`internal/webassembly`), over the runtime's WebAssembly engine
//! ([`crate::wasm`]). Modules, instances, memories, tables and globals are
//! objects of the prototypes the library registers, holding the engine's
//! structures; exported functions are native functions that call into the
//! engine, converting values as the JavaScript API specifies.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Kind, Object, Property, Slot};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::value::{Completion, Thrown, Value};
use crate::wasm::decode::{self, ExternKind, ImportDesc, Module};
use crate::wasm::exec::{
    self, Extern, Func, Global, Instance, Memory, RealmHost, Ref, Table, Trap, Val,
};
use crate::wasm::{FuncType, GlobalType, Limits, TableType, ValType};
use alloc::rc::Rc;
use core::cell::RefCell;

/// The prototypes and error constructors the library registers.
#[derive(Clone, Debug)]
pub(crate) struct State {
    module: Object,
    instance: Object,
    memory: Object,
    table: Object,
    global: Object,
    link_error: Value,
    runtime_error: Value,
    key: crate::symbol::Symbol,
}

/// What a binding object holds.
#[derive(Debug)]
enum Handle {
    Module(Rc<Module>),
    /// Held so the instance lives as long as its object.
    Instance(#[allow(dead_code)] Rc<Instance>),
    Memory(Rc<Memory>),
    Table(Rc<Table>),
    Global(Rc<Global>),
    Func(Rc<Func>),
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn state(realm: &Realm) -> Result<State, Thrown> {
    realm
        .wasm
        .clone()
        .ok_or_else(|| realm.throw(ErrorKind::Type, "WebAssembly is not initialized"))
}

fn handle(value: &Value) -> Option<Rc<dyn core::any::Any>> {
    match &value.as_object()?.data().kind {
        Kind::Native(native) => Some(native.clone()),
        _ => None,
    }
}

fn with_handle<T>(value: &Value, read: impl FnOnce(&Handle) -> Option<T>) -> Option<T> {
    let native = handle(value)?;
    read(native.downcast_ref::<Handle>()?)
}

fn module_of(realm: &Realm, value: &Value, context: &str) -> Result<Rc<Module>, Thrown> {
    with_handle(value, |handle| match handle {
        Handle::Module(module) => Some(module.clone()),
        _ => None,
    })
    .ok_or_else(|| {
        realm.throw(
            ErrorKind::Type,
            &format!("{context}: Argument 0 must be a WebAssembly.Module"),
        )
    })
}

fn memory_of(value: &Value) -> Option<Rc<Memory>> {
    with_handle(value, |handle| match handle {
        Handle::Memory(memory) => Some(memory.clone()),
        _ => None,
    })
}

fn table_of(value: &Value) -> Option<Rc<Table>> {
    with_handle(value, |handle| match handle {
        Handle::Table(table) => Some(table.clone()),
        _ => None,
    })
}

fn global_of(value: &Value) -> Option<Rc<Global>> {
    with_handle(value, |handle| match handle {
        Handle::Global(global) => Some(global.clone()),
        _ => None,
    })
}

/// The engine function an exported function calls, if a value is one.
fn exported_func(realm: &Realm, value: &Value) -> Option<Rc<Func>> {
    let key = &realm.wasm.as_ref()?.key;
    let Value::Object(object) = value else {
        return None;
    };
    let Slot::Data { value, .. } = object.private_property(key)?.slot else {
        return None;
    };
    with_handle(&value, |handle| match handle {
        Handle::Func(func) => Some(func.clone()),
        _ => None,
    })
}

fn native(prototype: &Object, handle: Handle) -> Object {
    Object::new(Some(prototype.clone()), Kind::Native(Rc::new(handle)))
}

/// The bytes of a `BufferSource`, copied.
fn buffer_source(value: &Value) -> Option<Vec<u8>> {
    if let Some(bytes) = crate::typed_array_builtin::buffer_of(value) {
        return Some(bytes.borrow().clone());
    }
    crate::typed_array_builtin::view_bytes(value)
}

fn error(realm: &mut Realm, constructor: &Value, message: &str) -> Thrown {
    match ops::construct(realm, constructor, &[Value::str(message)]) {
        Ok(error) => Thrown(error),
        Err(thrown) => thrown,
    }
}

/// The exception a trap throws: a `WebAssembly.RuntimeError`, a
/// `RangeError` for exhausted stack, or the JavaScript exception that
/// unwound through WebAssembly.
fn trap_error(realm: &mut Realm, trap: Trap) -> Thrown {
    match trap {
        Trap::Thrown(thrown) => thrown,
        Trap::StackOverflow => realm.throw(ErrorKind::Range, trap.message()),
        trap => match state(realm) {
            Ok(state) => error(realm, &state.runtime_error, trap.message()),
            Err(thrown) => thrown,
        },
    }
}

fn i64_of(value: &crate::bigint::BigInt) -> i64 {
    let truncated = value.as_int_n(64).expect("64-bit truncation fits");
    let magnitude = truncated.magnitude_u64().unwrap_or(0);
    if truncated.is_negative() {
        (magnitude as i64).wrapping_neg()
    } else {
        magnitude as i64
    }
}

/// `ToWebAssemblyValue`.
pub(crate) fn to_wasm(realm: &mut Realm, ty: ValType, value: &Value) -> Result<Val, Thrown> {
    Ok(match ty {
        ValType::I32 => Val::I32(ops::to_int32(realm, value)?),
        ValType::I64 => {
            let big = crate::bigint_builtin::to_big(realm, value)?;
            Val::I64(i64_of(&big))
        }
        ValType::F32 => Val::F32((ops::to_number(realm, value)? as f32).to_bits()),
        ValType::F64 => Val::F64(ops::to_number(realm, value)?.to_bits()),
        ValType::FuncRef => match value {
            Value::Null => Val::Ref(None),
            value => match exported_func(realm, value) {
                Some(func) => Val::Ref(Some(Ref::Func(func))),
                None => {
                    return Err(realm.throw(
                        ErrorKind::Type,
                        "type incompatibility when transforming from/to JS",
                    ));
                }
            },
        },
        ValType::ExternRef => match value {
            Value::Null => Val::Ref(None),
            value => Val::Ref(Some(Ref::Extern(value.clone()))),
        },
    })
}

/// `ToJSValue`.
pub(crate) fn to_js(realm: &mut Realm, value: Val) -> Value {
    match value {
        Val::I32(value) => Value::Number(f64::from(value)),
        Val::I64(value) => Value::BigInt(Rc::new(crate::bigint::BigInt::from_i64(value))),
        Val::F32(bits) => Value::Number(f64::from(f32::from_bits(bits))),
        Val::F64(bits) => Value::Number(f64::from_bits(bits)),
        Val::Ref(None) => Value::Null,
        Val::Ref(Some(Ref::Extern(value))) => value,
        Val::Ref(Some(Ref::Func(func))) => Value::Object(function_object(realm, &func)),
    }
}

/// The exported function for an engine function, made once.
fn function_object(realm: &mut Realm, func: &Rc<Func>) -> Object {
    if let Some(object) = func.export().borrow().clone() {
        return object;
    }
    let name = match &**func {
        Func::Wasm {
            instance, index, ..
        } => (instance.module.imported(ExternKind::Func) + *index as usize).to_string(),
        Func::Host { .. } => "0".to_owned(),
    };
    let target = func.clone();
    let object = realm.function(
        &name,
        func.ty().params.len() as u32,
        false,
        move |realm, _, args| call_export(realm, &target, args),
    );
    if let Some(state) = realm.wasm.clone() {
        object.set_private(
            &state.key,
            Property::hidden(Value::Object(Object::new(
                None,
                Kind::Native(Rc::new(Handle::Func(func.clone()))),
            ))),
        );
    }
    *func.export().borrow_mut() = Some(object.clone());
    object
}

fn call_export(realm: &mut Realm, func: &Rc<Func>, args: &[Value]) -> Completion {
    let ty = func.ty().clone();
    let mut values = Vec::with_capacity(ty.params.len());
    for (index, &param) in ty.params.iter().enumerate() {
        values.push(to_wasm(realm, param, &arg(args, index))?);
    }
    let results = exec::invoke(&mut RealmHost { realm: &mut *realm }, func, values)
        .map_err(|trap| trap_error(realm, trap))?;
    let mut results: Vec<Value> = results
        .into_iter()
        .map(|value| to_js(realm, value))
        .collect();
    Ok(match results.len() {
        0 => Value::Undefined,
        1 => results.pop().expect("a result"),
        _ => Value::Object(realm.array(results)),
    })
}

/// Calls an imported JavaScript function from WebAssembly.
pub(crate) fn call_host(
    realm: &mut Realm,
    callable: &Value,
    ty: &FuncType,
    args: Vec<Val>,
) -> Result<Vec<Val>, Trap> {
    let args: Vec<Value> = args.into_iter().map(|value| to_js(realm, value)).collect();
    let result = ops::call(realm, callable, Value::Undefined, &args).map_err(Trap::Thrown)?;
    match ty.results.len() {
        0 => Ok(Vec::new()),
        1 => Ok(vec![
            to_wasm(realm, ty.results[0], &result).map_err(Trap::Thrown)?,
        ]),
        count => {
            if !matches!(result, Value::Object(_)) {
                return Err(Trap::Thrown(
                    realm.throw(ErrorKind::Type, "multi-return value must be iterable"),
                ));
            }
            let values = crate::iter::collect(realm, &result).map_err(Trap::Thrown)?;
            if values.len() != count {
                return Err(Trap::Thrown(
                    realm.throw(ErrorKind::Type, "multi-return length mismatch"),
                ));
            }
            let mut converted = Vec::with_capacity(count);
            for (&ty, value) in ty.results.iter().zip(&values) {
                converted.push(to_wasm(realm, ty, value).map_err(Trap::Thrown)?);
            }
            Ok(converted)
        }
    }
}

fn memory_object(realm: &Realm, memory: &Rc<Memory>) -> Result<Object, Thrown> {
    if let Some(object) = memory.object.borrow().clone() {
        return Ok(object);
    }
    let object = native(&state(realm)?.memory, Handle::Memory(memory.clone()));
    *memory.object.borrow_mut() = Some(object.clone());
    Ok(object)
}

fn table_object(realm: &Realm, table: &Rc<Table>) -> Result<Object, Thrown> {
    if let Some(object) = table.object.borrow().clone() {
        return Ok(object);
    }
    let object = native(&state(realm)?.table, Handle::Table(table.clone()));
    *table.object.borrow_mut() = Some(object.clone());
    Ok(object)
}

fn global_object(realm: &Realm, global: &Rc<Global>) -> Result<Object, Thrown> {
    if let Some(object) = global.object.borrow().clone() {
        return Ok(object);
    }
    let object = native(&state(realm)?.global, Handle::Global(global.clone()));
    *global.object.borrow_mut() = Some(object.clone());
    Ok(object)
}

fn extern_value(realm: &mut Realm, value: Extern) -> Result<Value, Thrown> {
    Ok(Value::Object(match value {
        Extern::Func(func) => function_object(realm, &func),
        Extern::Table(table) => table_object(realm, &table)?,
        Extern::Memory(memory) => memory_object(realm, &memory)?,
        Extern::Global(global) => global_object(realm, &global)?,
    }))
}

fn text(realm: &mut Realm, value: &Value) -> Result<String, Thrown> {
    Ok(ops::to_string(realm, value)?.to_rust_lossy())
}

fn kind_name(kind: ExternKind) -> Value {
    Value::str(kind.name())
}

fn define(object: &Object, name: &str, value: Value) {
    object.define(Key::name(name), Property::data(value));
}

/// Reads an instance's imports from an import object, as the JavaScript
/// API's "read the imports" specifies.
fn read_imports(
    realm: &mut Realm,
    module: &Module,
    imports: &Value,
    context: &str,
) -> Result<Vec<Extern>, Thrown> {
    if !module.imports.is_empty() && !matches!(imports, Value::Object(_)) {
        let message = if matches!(imports, Value::Undefined) {
            format!("{context}: Imports argument must be present and must be an object")
        } else {
            format!("{context}: Argument 1 must be an object")
        };
        return Err(realm.throw(ErrorKind::Type, &message));
    }
    let state = state(realm)?;
    let mut resolved = Vec::with_capacity(module.imports.len());
    for (index, import) in module.imports.iter().enumerate() {
        let namespace = ops::get(realm, imports, &Key::name(&import.module))?;
        if !matches!(namespace, Value::Object(_)) {
            return Err(realm.throw(
                ErrorKind::Type,
                &format!(
                    "{context}: Import #{index} module=\"{}\": module is not an object or function",
                    import.module
                ),
            ));
        }
        let value = ops::get(realm, &namespace, &Key::name(&import.name))?;
        let link = |realm: &mut Realm, what: &str| {
            error(
                realm,
                &state.link_error,
                &format!(
                    "{context}: Import #{index} module=\"{}\" function=\"{}\": {what}",
                    import.module, import.name
                ),
            )
        };
        let resolved_value = match &import.desc {
            ImportDesc::Func(ty) => {
                if !crate::array_builtin::is_callable(&value) {
                    return Err(link(realm, "function import requires a callable"));
                }
                match exported_func(realm, &value) {
                    Some(func) => Extern::Func(func),
                    None => Extern::Func(Rc::new(Func::Host {
                        ty: module.types[*ty as usize].clone(),
                        callable: value,
                        export: RefCell::new(None),
                    })),
                }
            }
            ImportDesc::Global(ty) => {
                if let Some(global) = global_of(&value) {
                    Extern::Global(global)
                } else {
                    if ty.mutable {
                        return Err(link(
                            realm,
                            "imported mutable global must be a WebAssembly.Global object",
                        ));
                    }
                    let converted = match (ty.ty, &value) {
                        (ValType::I64, Value::BigInt(_)) => to_wasm(realm, ty.ty, &value)?,
                        (ValType::I64, _) => {
                            return Err(link(realm, "global import must be a BigInt value"));
                        }
                        (ValType::I32 | ValType::F32 | ValType::F64, Value::Number(_)) => {
                            to_wasm(realm, ty.ty, &value)?
                        }
                        (ValType::FuncRef | ValType::ExternRef, _) => {
                            match to_wasm(realm, ty.ty, &value) {
                                Ok(converted) => converted,
                                Err(_) => {
                                    return Err(link(
                                        realm,
                                        "imported funcref global must be null or a Wasm function",
                                    ));
                                }
                            }
                        }
                        _ => {
                            return Err(link(
                                realm,
                                "global import must be a number, valid Wasm reference, or WebAssembly.Global object",
                            ));
                        }
                    };
                    Extern::Global(Rc::new(Global::new(*ty, converted)))
                }
            }
            ImportDesc::Memory(_) => match memory_of(&value) {
                Some(memory) => Extern::Memory(memory),
                None => {
                    return Err(link(
                        realm,
                        "memory import must be a WebAssembly.Memory object",
                    ));
                }
            },
            ImportDesc::Table(_) => match table_of(&value) {
                Some(table) => Extern::Table(table),
                None => return Err(link(realm, "table import requires a WebAssembly.Table")),
            },
        };
        resolved.push(resolved_value);
    }
    Ok(resolved)
}

fn instantiate(realm: &mut Realm, args: &[Value]) -> Completion {
    let context = text(realm, &arg(args, 2))?;
    let module = module_of(realm, &arg(args, 0), &context)?;
    let imports = read_imports(realm, &module, &arg(args, 1), &context)?;
    let state = state(realm)?;
    let instance = match exec::instantiate(
        &mut RealmHost { realm: &mut *realm },
        module.clone(),
        imports,
    ) {
        Ok(instance) => instance,
        Err(exec::InstantiateError::Link(message)) => {
            return Err(error(
                realm,
                &state.link_error,
                &format!("{context}: {message}"),
            ));
        }
        Err(exec::InstantiateError::Trap(trap)) => return Err(trap_error(realm, trap)),
    };
    let exports = Object::new(None, Kind::Ordinary);
    for export in &module.exports {
        if export.kind == ExternKind::Tag {
            continue;
        }
        let value = extern_value(realm, instance.export(export.kind, export.index))?;
        exports.define(
            Key::name(&export.name),
            Property {
                slot: Slot::Data {
                    value,
                    writable: true,
                },
                enumerable: true,
                configurable: true,
            },
        );
    }
    let object = native(&state.instance, Handle::Instance(instance));
    object.define(
        Key::name("exports"),
        Property::hidden(Value::Object(exports)),
    );
    Ok(Value::Object(object))
}

fn compile(realm: &mut Realm, args: &[Value]) -> Completion {
    let Some(bytes) = buffer_source(&arg(args, 0)) else {
        return Ok(Value::Undefined);
    };
    match decode::decode(bytes) {
        Ok(module) => {
            let state = state(realm)?;
            Ok(Value::Object(native(
                &state.module,
                Handle::Module(Rc::new(module)),
            )))
        }
        Err(error) => Ok(Value::str(&error.describe())),
    }
}

fn limits_of(realm: &mut Realm, args: &[Value], first: usize) -> Result<Limits, Thrown> {
    let min = ops::to_uint32(realm, &arg(args, first))?;
    let max = match arg(args, first + 1) {
        Value::Undefined => None,
        value => Some(ops::to_uint32(realm, &value)?),
    };
    Ok(Limits { min, max })
}

fn type_of(realm: &mut Realm, value: &Value) -> Result<ValType, Thrown> {
    let name = text(realm, value)?;
    ValType::parse(&name).ok_or_else(|| realm.throw(ErrorKind::Type, "invalid value type"))
}

fn func_type_value(realm: &Realm, ty: &FuncType) -> Value {
    let list = |types: &[ValType]| {
        Value::Object(realm.array(types.iter().map(|ty| Value::str(ty.name())).collect()))
    };
    let object = realm.object();
    define(&object, "parameters", list(&ty.params));
    define(&object, "results", list(&ty.results));
    Value::Object(object)
}

fn limits_value(realm: &Realm, limits: &Limits, element: Option<ValType>) -> Value {
    let object = realm.object();
    if let Some(element) = element {
        define(&object, "element", Value::str(element.name()));
    }
    define(&object, "minimum", Value::Number(f64::from(limits.min)));
    if let Some(max) = limits.max {
        define(&object, "maximum", Value::Number(f64::from(max)));
    }
    Value::Object(object)
}

fn global_type_value(realm: &Realm, ty: &GlobalType) -> Value {
    let object = realm.object();
    define(&object, "value", Value::str(ty.ty.name()));
    define(&object, "mutable", Value::Boolean(ty.mutable));
    Value::Object(object)
}

fn require<T>(realm: &Realm, value: Option<T>, message: &str) -> Result<T, Thrown> {
    value.ok_or_else(|| realm.throw(ErrorKind::Type, message))
}

/// `internalBinding('wasm')`.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let method = |realm: &mut Realm,
                  name: &str,
                  length: u32,
                  code: fn(&mut Realm, &[Value]) -> Completion| {
        let function = realm.function(name, length, false, move |realm, _, args| code(realm, args));
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    };
    method(realm, "init", 1, |realm, args| {
        let options = arg(args, 0);
        let mut read = |name: &str| ops::get(realm, &options, &Key::name(name));
        let object = |value: Value| match value {
            Value::Object(object) => Some(object),
            _ => None,
        };
        let module = object(read("Module")?);
        let instance = object(read("Instance")?);
        let memory = object(read("Memory")?);
        let table = object(read("Table")?);
        let global = object(read("Global")?);
        let link_error = read("LinkError")?;
        let runtime_error = read("RuntimeError")?;
        let (Some(module), Some(instance), Some(memory), Some(table), Some(global)) =
            (module, instance, memory, table, global)
        else {
            return Err(realm.throw(ErrorKind::Type, "WebAssembly prototypes are required"));
        };
        realm.wasm = Some(State {
            module,
            instance,
            memory,
            table,
            global,
            link_error,
            runtime_error,
            key: crate::symbol::Symbol::private("nativeboat.wasm.function"),
        });
        Ok(Value::Undefined)
    });
    method(realm, "compile", 1, compile);
    method(realm, "validate", 1, |realm, args| {
        let Some(bytes) = buffer_source(&arg(args, 0)) else {
            return Err(realm.throw(
                ErrorKind::Type,
                "WebAssembly.validate(): Argument 0 must be a buffer source",
            ));
        };
        Ok(Value::Boolean(decode::decode(bytes).is_ok()))
    });
    method(realm, "isModule", 1, |_, args| {
        Ok(Value::Boolean(
            with_handle(&arg(args, 0), |handle| {
                matches!(handle, Handle::Module(_)).then_some(())
            })
            .is_some(),
        ))
    });
    method(realm, "moduleImports", 1, |realm, args| {
        let module = module_of(realm, &arg(args, 0), "WebAssembly.Module.imports()")?;
        let list = module
            .imports
            .iter()
            .map(|import| {
                let object = realm.object();
                define(&object, "module", Value::str(&import.module));
                define(&object, "name", Value::str(&import.name));
                define(&object, "kind", kind_name(import.desc.kind()));
                let ty = match &import.desc {
                    ImportDesc::Func(ty) => func_type_value(realm, &module.types[*ty as usize]),
                    ImportDesc::Table(ty) => limits_value(realm, &ty.limits, Some(ty.element)),
                    ImportDesc::Memory(limits) => limits_value(realm, limits, None),
                    ImportDesc::Global(ty) => global_type_value(realm, ty),
                };
                define(&object, "type", ty);
                Value::Object(object)
            })
            .collect();
        Ok(Value::Object(realm.array(list)))
    });
    method(realm, "moduleExports", 1, |realm, args| {
        let module = module_of(realm, &arg(args, 0), "WebAssembly.Module.exports()")?;
        let tables = module.table_types();
        let globals = module.global_types();
        let imported_memory = module.imports.iter().find_map(|import| match import.desc {
            ImportDesc::Memory(limits) => Some(limits),
            _ => None,
        });
        let list = module
            .exports
            .iter()
            .map(|export| {
                let object = realm.object();
                define(&object, "name", Value::str(&export.name));
                define(&object, "kind", kind_name(export.kind));
                let index = export.index as usize;
                let ty = match export.kind {
                    ExternKind::Func => module
                        .func_type(export.index)
                        .map(|ty| func_type_value(realm, ty)),
                    ExternKind::Table => tables
                        .get(index)
                        .map(|ty| limits_value(realm, &ty.limits, Some(ty.element))),
                    ExternKind::Memory => imported_memory
                        .or_else(|| module.memories.first().copied())
                        .map(|limits| limits_value(realm, &limits, None)),
                    ExternKind::Global => globals.get(index).map(|ty| global_type_value(realm, ty)),
                    ExternKind::Tag => None,
                };
                if let Some(ty) = ty {
                    define(&object, "type", ty);
                }
                Value::Object(object)
            })
            .collect();
        Ok(Value::Object(realm.array(list)))
    });
    method(realm, "customSections", 2, |realm, args| {
        let module = module_of(realm, &arg(args, 0), "WebAssembly.Module.customSections()")?;
        let name = text(realm, &arg(args, 1))?;
        let prototype = realm.array_buffer_prototype.clone();
        let list = module
            .customs
            .iter()
            .filter(|(section, _)| *section == name)
            .map(|(_, range)| {
                let bytes = Rc::new(RefCell::new(module.bytes[range.clone()].to_vec()));
                Value::Object(Object::new(
                    Some(prototype.clone()),
                    Kind::ArrayBuffer(bytes),
                ))
            })
            .collect();
        Ok(Value::Object(realm.array(list)))
    });
    method(realm, "instantiate", 3, instantiate);
    method(realm, "newMemory", 2, |realm, args| {
        let limits = limits_of(realm, args, 0)?;
        if limits.min > exec::MAX_PAGES || limits.max.is_some_and(|max| max > exec::MAX_PAGES) {
            return Err(realm.throw(
                ErrorKind::Range,
                "WebAssembly.Memory(): Property 'initial': value is above the upper bound",
            ));
        }
        let Some(memory) = Memory::new(limits) else {
            return Err(realm.throw(
                ErrorKind::Range,
                "WebAssembly.Memory(): could not allocate memory",
            ));
        };
        Ok(Value::Object(memory_object(realm, &Rc::new(memory))?))
    });
    method(realm, "memoryBuffer", 1, |realm, args| {
        let memory = require(
            realm,
            memory_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Memory",
        )?;
        if let Some(buffer) = memory.buffer.borrow().clone() {
            return Ok(Value::Object(buffer));
        }
        let buffer = Object::new(
            Some(realm.array_buffer_prototype.clone()),
            Kind::ArrayBuffer(memory.bytes.clone()),
        );
        *memory.buffer.borrow_mut() = Some(buffer.clone());
        Ok(Value::Object(buffer))
    });
    method(realm, "memoryGrow", 2, |realm, args| {
        let memory = require(
            realm,
            memory_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Memory",
        )?;
        let delta = ops::to_uint32(realm, &arg(args, 1))?;
        match memory.grow(delta) {
            Some(old) => {
                // Growing replaces the buffer even by zero pages.
                *memory.buffer.borrow_mut() = None;
                Ok(Value::Number(f64::from(old)))
            }
            None => Err(realm.throw(
                ErrorKind::Range,
                "WebAssembly.Memory.grow(): Maximum memory size exceeded",
            )),
        }
    });
    method(realm, "memoryType", 1, |realm, args| {
        let memory = require(
            realm,
            memory_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Memory",
        )?;
        let limits = Limits {
            min: memory.pages(),
            max: memory.limits.borrow().max,
        };
        Ok(limits_value(realm, &limits, None))
    });
    method(realm, "newTable", 4, |realm, args| {
        let element = type_of(realm, &arg(args, 0))?;
        if !element.is_reference() {
            return Err(realm.throw(
                ErrorKind::Type,
                "WebAssembly.Table(): Descriptor property 'element' must be a WebAssembly reference type",
            ));
        }
        let limits = limits_of(realm, args, 1)?;
        if limits.min > 10_000_000 {
            return Err(realm.throw(
                ErrorKind::Range,
                "WebAssembly.Table(): Property 'initial': value is above the upper bound 10000000",
            ));
        }
        let initial = match arg(args, 3) {
            Value::Undefined if element == ValType::FuncRef => Val::Ref(None),
            value => to_wasm(realm, element, &value)?,
        };
        let Val::Ref(initial) = initial else {
            unreachable!("reference types convert to references");
        };
        let table = Table::new(TableType { element, limits }, initial);
        Ok(Value::Object(table_object(realm, &Rc::new(table))?))
    });
    method(realm, "tableLength", 1, |realm, args| {
        let table = require(
            realm,
            table_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Table",
        )?;
        Ok(Value::Number(f64::from(table.size())))
    });
    method(realm, "tableGet", 2, |realm, args| {
        let table = require(
            realm,
            table_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Table",
        )?;
        let index = ops::to_uint32(realm, &arg(args, 1))?;
        let element = table.elements.borrow().get(index as usize).cloned();
        let Some(element) = element else {
            return Err(realm.throw(
                ErrorKind::Range,
                &format!(
                    "WebAssembly.Table.get(): invalid address {index} in {} table of size {}",
                    table.ty.element.name(),
                    table.size()
                ),
            ));
        };
        Ok(to_js(realm, Val::Ref(element)))
    });
    method(realm, "tableSet", 3, |realm, args| {
        let table = require(
            realm,
            table_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Table",
        )?;
        let index = ops::to_uint32(realm, &arg(args, 1))?;
        let value = match (args.len() > 2, table.ty.element) {
            (false, ValType::FuncRef) => Val::Ref(None),
            _ => to_wasm(realm, table.ty.element, &arg(args, 2))?,
        };
        let Val::Ref(value) = value else {
            unreachable!("reference types convert to references");
        };
        let size = table.size();
        if index >= size {
            return Err(realm.throw(
                ErrorKind::Range,
                &format!(
                    "WebAssembly.Table.set(): invalid address {index} in {} table of size {size}",
                    table.ty.element.name()
                ),
            ));
        }
        table.elements.borrow_mut()[index as usize] = value;
        Ok(Value::Undefined)
    });
    method(realm, "tableGrow", 3, |realm, args| {
        let table = require(
            realm,
            table_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Table",
        )?;
        let delta = ops::to_uint32(realm, &arg(args, 1))?;
        let initial = match (args.len() > 2, table.ty.element) {
            (false, ValType::FuncRef) => Val::Ref(None),
            _ => to_wasm(realm, table.ty.element, &arg(args, 2))?,
        };
        let Val::Ref(initial) = initial else {
            unreachable!("reference types convert to references");
        };
        match table.grow(delta, initial) {
            Some(old) => Ok(Value::Number(f64::from(old))),
            None => Err(realm.throw(
                ErrorKind::Range,
                &format!("WebAssembly.Table.grow(): failed to grow table by {delta}"),
            )),
        }
    });
    method(realm, "tableType", 1, |realm, args| {
        let table = require(
            realm,
            table_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Table",
        )?;
        let limits = Limits {
            min: table.size(),
            max: table.ty.limits.max,
        };
        Ok(limits_value(realm, &limits, Some(table.ty.element)))
    });
    method(realm, "newGlobal", 3, |realm, args| {
        let ty = type_of(realm, &arg(args, 0))?;
        let mutable = ops::to_boolean(&arg(args, 1));
        // A missing or undefined value is the type's default, which for
        // externref is undefined itself.
        let value = match arg(args, 2) {
            Value::Undefined if ty != ValType::ExternRef => Val::default_of(ty),
            value => to_wasm(realm, ty, &value)?,
        };
        let global = Global::new(GlobalType { ty, mutable }, value);
        Ok(Value::Object(global_object(realm, &Rc::new(global))?))
    });
    method(realm, "globalGet", 1, |realm, args| {
        let global = require(
            realm,
            global_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Global",
        )?;
        let value = global.value.borrow().clone();
        Ok(to_js(realm, value))
    });
    method(realm, "globalSet", 2, |realm, args| {
        let global = require(
            realm,
            global_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Global",
        )?;
        if !global.ty.mutable {
            return Err(realm.throw(
                ErrorKind::Type,
                "WebAssembly.Global.set(): Can't set the value of an immutable global.",
            ));
        }
        let value = to_wasm(realm, global.ty.ty, &arg(args, 1))?;
        *global.value.borrow_mut() = value;
        Ok(Value::Undefined)
    });
    method(realm, "globalType", 1, |realm, args| {
        let global = require(
            realm,
            global_of(&arg(args, 0)),
            "Receiver is not a WebAssembly.Global",
        )?;
        Ok(global_type_value(realm, &global.ty))
    });
    method(realm, "functionType", 1, |realm, args| {
        let func = require(
            realm,
            exported_func(realm, &arg(args, 0)),
            "Argument 0 must be a WebAssembly exported function",
        )?;
        Ok(func_type_value(realm, func.ty()))
    });
    exports
}
