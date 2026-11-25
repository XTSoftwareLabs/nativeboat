use super::decode::{self, CompileError};
use super::exec::{self, Extern, Func, Host, Memory, Trap, Val};
use super::{FuncType, Limits, ValType};
use crate::value::Value;
use alloc::rc::Rc;
use core::cell::RefCell;

const I32: u8 = 0x7F;
const I64: u8 = 0x7E;
const F32: u8 = 0x7D;
const F64: u8 = 0x7C;
const FUNCREF: u8 = 0x70;

fn unsigned(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn signed(mut value: i64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        out.push(if done { byte } else { byte | 0x80 });
        if done {
            return;
        }
    }
}

fn u(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    unsigned(value, &mut out);
    out
}

fn name(text: &str) -> Vec<u8> {
    let mut out = u(text.len() as u64);
    out.extend_from_slice(text.as_bytes());
    out
}

fn vector(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = u(items.len() as u64);
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

/// A module of `(id, content)` sections.
fn module(sections: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"\0asm\x01\0\0\0".to_vec();
    for (id, content) in sections {
        out.push(*id);
        unsigned(content.len() as u64, &mut out);
        out.extend_from_slice(content);
    }
    out
}

fn func_type(params: &[u8], results: &[u8]) -> Vec<u8> {
    let mut out = vec![0x60];
    out.extend(u(params.len() as u64));
    out.extend_from_slice(params);
    out.extend(u(results.len() as u64));
    out.extend_from_slice(results);
    out
}

/// A function body: local groups and instructions (without the final
/// `end`).
fn body(locals: &[(u32, u8)], code: &[u8]) -> Vec<u8> {
    let mut content = u(locals.len() as u64);
    for (count, ty) in locals {
        content.extend(u(u64::from(*count)));
        content.push(*ty);
    }
    content.extend_from_slice(code);
    content.push(0x0B);
    let mut out = u(content.len() as u64);
    out.extend(content);
    out
}

fn export(name_text: &str, kind: u8, index: u32) -> Vec<u8> {
    let mut out = name(name_text);
    out.push(kind);
    out.extend(u(u64::from(index)));
    out
}

fn i32_const(value: i32) -> Vec<u8> {
    let mut out = vec![0x41];
    signed(i64::from(value), &mut out);
    out
}

fn i64_const(value: i64) -> Vec<u8> {
    let mut out = vec![0x42];
    signed(value, &mut out);
    out
}

/// A module of functions with the given types, exported by name in order.
fn functions(types: &[Vec<u8>], funcs: &[(u32, Vec<u8>)], names: &[&str]) -> Vec<u8> {
    let declarations: Vec<Vec<u8>> = funcs.iter().map(|(ty, _)| u(u64::from(*ty))).collect();
    let exports: Vec<Vec<u8>> = names
        .iter()
        .enumerate()
        .map(|(index, name)| export(name, 0, index as u32))
        .collect();
    let bodies: Vec<Vec<u8>> = funcs.iter().map(|(_, body)| body.clone()).collect();
    module(&[
        (1, vector(types)),
        (3, vector(&declarations)),
        (7, vector(&exports)),
        (10, vector(&bodies)),
    ])
}

/// Host functions for tests: each import is a Rust function of its
/// arguments.
struct TestHost {
    calls: Rc<RefCell<Vec<Vec<Val>>>>,
    reply: fn(&[Val]) -> Result<Vec<Val>, Trap>,
}

impl Host for TestHost {
    fn call(&mut self, _: &Value, _: &FuncType, args: Vec<Val>) -> Result<Vec<Val>, Trap> {
        let reply = (self.reply)(&args);
        self.calls.borrow_mut().push(args);
        reply
    }
}

fn host() -> TestHost {
    TestHost {
        calls: Rc::new(RefCell::new(Vec::new())),
        reply: |_| Ok(Vec::new()),
    }
}

fn instance(bytes: Vec<u8>, imports: Vec<Extern>) -> Rc<exec::Instance> {
    let module = Rc::new(decode::decode(bytes).expect("a valid module"));
    match exec::instantiate(&mut host(), module, imports) {
        Ok(instance) => instance,
        Err(error) => panic!("instantiation failed: {error:?}"),
    }
}

fn call(instance: &exec::Instance, name: &str, args: Vec<Val>) -> Result<Vec<Val>, Trap> {
    let export = instance
        .module
        .exports
        .iter()
        .find(|export| export.name == name)
        .expect("the export");
    let func = instance.funcs()[export.index as usize].clone();
    exec::invoke(&mut host(), &func, args)
}

fn i32_result(result: Result<Vec<Val>, Trap>) -> i32 {
    match result.expect("no trap").as_slice() {
        [Val::I32(value)] => *value,
        other => panic!("expected one i32, got {other:?}"),
    }
}

fn i64_result(result: Result<Vec<Val>, Trap>) -> i64 {
    match result.expect("no trap").as_slice() {
        [Val::I64(value)] => *value,
        other => panic!("expected one i64, got {other:?}"),
    }
}

fn trap_message(result: Result<Vec<Val>, Trap>) -> &'static str {
    match result {
        Err(trap) => trap.message(),
        Ok(values) => panic!("expected a trap, got {values:?}"),
    }
}

#[test]
fn recursive_and_looping_functions_compute() {
    // fac(n) = n == 0 ? 1 : n * fac(n - 1), over i64.
    let mut fac = vec![0x20, 0, 0x50, 0x04, I64];
    fac.extend(i64_const(1));
    fac.push(0x05);
    fac.extend([0x20, 0]);
    fac.extend([0x20, 0]);
    fac.extend(i64_const(1));
    fac.extend([0x7D, 0x10, 0, 0x7E, 0x0B]);
    // sum(n): a loop adding 1..=n into a local.
    let mut sum = vec![0x02, 0x40, 0x03, 0x40, 0x20, 0, 0x45, 0x0D, 1];
    sum.extend([0x20, 1, 0x20, 0, 0x6A, 0x21, 1]);
    sum.extend([0x20, 0]);
    sum.extend(i32_const(1));
    sum.extend([0x6B, 0x21, 0, 0x0C, 0, 0x0B, 0x0B, 0x20, 1]);
    let bytes = functions(
        &[func_type(&[I64], &[I64]), func_type(&[I32], &[I32])],
        &[(0, body(&[], &fac)), (1, body(&[(1, I32)], &sum))],
        &["fac", "sum"],
    );
    let instance = instance(bytes, Vec::new());
    assert_eq!(
        i64_result(call(&instance, "fac", vec![Val::I64(20)])),
        2_432_902_008_176_640_000
    );
    assert_eq!(
        i32_result(call(&instance, "sum", vec![Val::I32(100)])),
        5050
    );
    assert_eq!(i32_result(call(&instance, "sum", vec![Val::I32(0)])), 0);
}

#[test]
fn branch_tables_and_blocks_with_values_move_results() {
    // classify(x): br_table over four blocks, each returning a result.
    let mut code = vec![
        0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x20, 0, 0x0E, 3, 0, 1, 2, 3, 0x0B,
    ];
    code.extend(i32_const(10));
    code.extend([0x0F, 0x0B]);
    code.extend(i32_const(20));
    code.extend([0x0F, 0x0B]);
    code.extend(i32_const(30));
    code.extend([0x0F, 0x0B]);
    code.extend(i32_const(99));
    // A block that branches out with a value above extra operands.
    let mut drop_extra = vec![0x02, I32];
    drop_extra.extend(i32_const(1));
    drop_extra.extend(i32_const(2));
    drop_extra.extend(i32_const(42));
    drop_extra.extend([0x0C, 0, 0x0B]);
    // A multi-value block type: (i32 i32) -> (i32) via type 2.
    let mut multi = i32_const(7);
    multi.extend(i32_const(5));
    multi.extend([0x02, 2, 0x6B, 0x0B]);
    let bytes = functions(
        &[
            func_type(&[I32], &[I32]),
            func_type(&[], &[I32]),
            func_type(&[I32, I32], &[I32]),
        ],
        &[
            (0, body(&[], &code)),
            (1, body(&[], &drop_extra)),
            (1, body(&[], &multi)),
        ],
        &["classify", "extra", "multi"],
    );
    let instance = instance(bytes, Vec::new());
    let classify = |value| i32_result(call(&instance, "classify", vec![Val::I32(value)]));
    assert_eq!(
        [
            classify(0),
            classify(1),
            classify(2),
            classify(3),
            classify(-1)
        ],
        [10, 20, 30, 99, 99]
    );
    assert_eq!(i32_result(call(&instance, "extra", Vec::new())), 42);
    assert_eq!(i32_result(call(&instance, "multi", Vec::new())), 2);
}

#[test]
fn if_else_and_select_choose_values() {
    let mut code = vec![0x20, 0, 0x04, I32];
    code.extend(i32_const(1));
    code.push(0x05);
    code.extend(i32_const(2));
    code.push(0x0B);
    code.extend(i32_const(100));
    code.extend(i32_const(200));
    code.extend([0x20, 0, 0x1B, 0x6A]);
    let bytes = functions(
        &[func_type(&[I32], &[I32])],
        &[(0, body(&[], &code))],
        &["pick"],
    );
    let instance = instance(bytes, Vec::new());
    assert_eq!(i32_result(call(&instance, "pick", vec![Val::I32(1)])), 101);
    assert_eq!(i32_result(call(&instance, "pick", vec![Val::I32(0)])), 202);
}

fn memory_module() -> Vec<u8> {
    // load(a) = i32.load(a); store(a, v); grow(n); size(); fill/copy.
    let load = body(&[], &[0x20, 0, 0x28, 2, 0]);
    let store = body(&[], &[0x20, 0, 0x20, 1, 0x36, 2, 0]);
    let grow = body(&[], &[0x20, 0, 0x40, 0]);
    let size = body(&[], &[0x3F, 0]);
    let load8 = body(&[], &[0x20, 0, 0x2C, 0, 0]);
    let fill = body(&[], &[0x20, 0, 0x20, 1, 0x20, 2, 0xFC, 11, 0]);
    let copy = body(&[], &[0x20, 0, 0x20, 1, 0x20, 2, 0xFC, 10, 0, 0]);
    let init = body(&[], &[0x20, 0, 0x41, 0, 0x41, 3, 0xFC, 8, 1, 0, 0xFC, 9, 1]);
    let types = vec![
        func_type(&[I32], &[I32]),
        func_type(&[I32, I32], &[]),
        func_type(&[], &[I32]),
        func_type(&[I32, I32, I32], &[]),
        func_type(&[I32], &[]),
    ];
    let declarations: Vec<Vec<u8>> = [0, 1, 0, 2, 0, 3, 3, 4].iter().map(|ty| u(*ty)).collect();
    let names = [
        "load", "store", "grow", "size", "load8", "fill", "copy", "init",
    ];
    let mut exports: Vec<Vec<u8>> = names
        .iter()
        .enumerate()
        .map(|(index, name)| export(name, 0, index as u32))
        .collect();
    exports.push(export("memory", 2, 0));
    let mut active = vec![0];
    active.extend(i32_const(16));
    active.push(0x0B);
    active.extend(name("\x01\x02\x03\x04"));
    let mut passive = vec![1];
    passive.extend(name("abc"));
    module(&[
        (1, vector(&types)),
        (3, vector(&declarations)),
        (5, vector(&[vec![1, 1, 3]])),
        (7, vector(&exports)),
        (12, u(2)),
        (
            10,
            vector(&[load, store, grow, size, load8, fill, copy, init]),
        ),
        (11, vector(&[active, passive])),
    ])
}

#[test]
fn memories_load_store_grow_and_copy() {
    let instance = instance(memory_module(), Vec::new());
    let memory = instance.memory.clone().expect("a memory");
    assert_eq!(
        i32_result(call(&instance, "load", vec![Val::I32(16)])),
        0x0403_0201
    );
    call(&instance, "store", vec![Val::I32(100), Val::I32(-2)]).expect("a store");
    assert_eq!(&memory.bytes.borrow()[100..104], &[0xFE, 0xFF, 0xFF, 0xFF]);
    assert_eq!(
        i32_result(call(&instance, "load8", vec![Val::I32(100)])),
        -2
    );
    call(
        &instance,
        "fill",
        vec![Val::I32(200), Val::I32(7), Val::I32(3)],
    )
    .expect("a fill");
    call(
        &instance,
        "copy",
        vec![Val::I32(201), Val::I32(16), Val::I32(4)],
    )
    .expect("a copy");
    assert_eq!(&memory.bytes.borrow()[200..206], &[7, 1, 2, 3, 4, 0]);
    call(&instance, "init", vec![Val::I32(300)]).expect("an init");
    assert_eq!(&memory.bytes.borrow()[300..303], b"abc");
    // The segment was dropped: initializing again with a length traps.
    assert_eq!(
        trap_message(call(&instance, "init", vec![Val::I32(300)])),
        "memory access out of bounds"
    );
    assert_eq!(i32_result(call(&instance, "size", Vec::new())), 1);
    assert_eq!(i32_result(call(&instance, "grow", vec![Val::I32(1)])), 1);
    assert_eq!(i32_result(call(&instance, "grow", vec![Val::I32(5)])), -1);
    assert_eq!(i32_result(call(&instance, "size", Vec::new())), 2);
    assert_eq!(memory.bytes.borrow().len(), 2 * exec::PAGE);
    assert_eq!(
        trap_message(call(&instance, "load", vec![Val::I32(2 * 65536 - 3)])),
        "memory access out of bounds"
    );
    assert_eq!(
        trap_message(call(
            &instance,
            "fill",
            vec![Val::I32(2 * 65536 - 1), Val::I32(0), Val::I32(2)]
        )),
        "memory access out of bounds"
    );
}

#[test]
fn imported_memories_are_shared() {
    let load = body(&[], &[0x20, 0, 0x28, 2, 0]);
    let mut import = name("env");
    import.extend(name("memory"));
    import.extend([2, 0, 1]);
    let bytes = module(&[
        (1, vector(&[func_type(&[I32], &[I32])])),
        (2, vector(&[import])),
        (3, vector(&[u(0)])),
        (7, vector(&[export("load", 0, 0)])),
        (10, vector(&[load])),
    ]);
    let memory = Rc::new(Memory::new(Limits { min: 1, max: None }).expect("a memory"));
    memory.bytes.borrow_mut()[8..12].copy_from_slice(&77u32.to_le_bytes());
    let instance = instance(bytes, vec![Extern::Memory(memory)]);
    assert_eq!(i32_result(call(&instance, "load", vec![Val::I32(8)])), 77);
}

#[test]
fn indirect_calls_check_signatures() {
    // Table of [add1, neg, (null)]; dispatch(i, x) = call_indirect type 0.
    let add1 = body(&[], &[0x20, 0, 0x41, 1, 0x6A]);
    let neg = body(&[], &[0x41, 0, 0x20, 0, 0x6B]);
    let other = body(&[], &[]);
    let dispatch = body(&[], &[0x20, 1, 0x20, 0, 0x11, 0, 0]);
    let mut elem = vec![0];
    elem.extend(i32_const(0));
    elem.push(0x0B);
    elem.extend(vector(&[u(0), u(1), u(3)]));
    let bytes = module(&[
        (
            1,
            vector(&[
                func_type(&[I32], &[I32]),
                func_type(&[I32, I32], &[I32]),
                func_type(&[], &[]),
            ]),
        ),
        (3, vector(&[u(0), u(0), u(1), u(2)])),
        (4, vector(&[vec![FUNCREF, 0, 4]])),
        (7, vector(&[export("dispatch", 0, 2)])),
        (9, vector(&[elem])),
        (10, vector(&[add1, neg, dispatch, other])),
    ]);
    let instance = instance(bytes, Vec::new());
    let dispatch = |index, value| {
        call(
            &instance,
            "dispatch",
            vec![Val::I32(index), Val::I32(value)],
        )
    };
    assert_eq!(i32_result(dispatch(0, 41)), 42);
    assert_eq!(i32_result(dispatch(1, 5)), -5);
    assert_eq!(
        trap_message(dispatch(2, 0)),
        "null function or function signature mismatch"
    );
    assert_eq!(
        trap_message(dispatch(3, 0)),
        "null function or function signature mismatch"
    );
    assert_eq!(trap_message(dispatch(4, 0)), "table index is out of bounds");
}

#[test]
fn host_functions_receive_arguments_and_return_results() {
    let mut import = name("env");
    import.extend(name("twice"));
    import.extend([0, 0]);
    // run(x) = twice(x) + 1
    let run = body(&[], &[0x20, 0, 0x10, 0, 0x41, 1, 0x6A]);
    let bytes = module(&[
        (1, vector(&[func_type(&[I32], &[I32])])),
        (2, vector(&[import])),
        (3, vector(&[u(0)])),
        (7, vector(&[export("run", 0, 1)])),
        (10, vector(&[run])),
    ]);
    let module = Rc::new(decode::decode(bytes).expect("a valid module"));
    let twice = Rc::new(Func::Host {
        ty: FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        },
        callable: Value::Undefined,
        export: RefCell::new(None),
    });
    let mut host = TestHost {
        calls: Rc::new(RefCell::new(Vec::new())),
        reply: |args| match args {
            [Val::I32(value)] => Ok(vec![Val::I32(value * 2)]),
            _ => Err(Trap::Unreachable),
        },
    };
    let instance = match exec::instantiate(&mut host, module, vec![Extern::Func(twice)]) {
        Ok(instance) => instance,
        Err(error) => panic!("{error:?}"),
    };
    let func = instance.funcs()[1].clone();
    let result = exec::invoke(&mut host, &func, vec![Val::I32(20)]);
    assert_eq!(i32_result(result), 41);
    assert_eq!(host.calls.borrow().len(), 1);
}

#[test]
fn linking_checks_import_types() {
    let mut import = name("env");
    import.extend(name("f"));
    import.extend([0, 0]);
    let bytes = module(&[
        (1, vector(&[func_type(&[I32], &[])])),
        (2, vector(&[import])),
    ]);
    let module = Rc::new(decode::decode(bytes).expect("a valid module"));
    let wrong = Rc::new(Func::Host {
        ty: FuncType {
            params: vec![],
            results: vec![],
        },
        callable: Value::Undefined,
        export: RefCell::new(None),
    });
    match exec::instantiate(&mut host(), module, vec![Extern::Func(wrong)]) {
        Err(exec::InstantiateError::Link(message)) => assert_eq!(
            message,
            "Import #0 module=\"env\" function=\"f\": imported function does not match the expected type"
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn traps_report_v8_messages() {
    let divide = body(&[], &[0x20, 0, 0x20, 1, 0x6D]);
    let remainder = body(&[], &[0x20, 0, 0x20, 1, 0x70]);
    let unreachable = body(&[], &[0x00]);
    let forever = body(&[], &[0x10, 3]);
    let truncate = body(&[], &[0x20, 0, 0xAA]);
    let saturate = body(&[], &[0x20, 0, 0xFC, 2]);
    let bytes = functions(
        &[
            func_type(&[I32, I32], &[I32]),
            func_type(&[], &[]),
            func_type(&[F64], &[I32]),
        ],
        &[
            (0, divide),
            (0, remainder),
            (1, unreachable),
            (1, forever),
            (2, truncate),
            (2, saturate),
        ],
        &[
            "divide",
            "remainder",
            "unreachable",
            "forever",
            "truncate",
            "saturate",
        ],
    );
    let instance = instance(bytes, Vec::new());
    let pair = |a, b| vec![Val::I32(a), Val::I32(b)];
    assert_eq!(i32_result(call(&instance, "divide", pair(-7, 2))), -3);
    assert_eq!(
        trap_message(call(&instance, "divide", pair(1, 0))),
        "divide by zero"
    );
    assert_eq!(
        trap_message(call(&instance, "divide", pair(i32::MIN, -1))),
        "divide result unrepresentable"
    );
    assert_eq!(
        trap_message(call(&instance, "remainder", pair(1, 0))),
        "remainder by zero"
    );
    assert_eq!(
        trap_message(call(&instance, "unreachable", Vec::new())),
        "unreachable"
    );
    assert_eq!(
        trap_message(call(&instance, "forever", Vec::new())),
        "Maximum call stack size exceeded"
    );
    let float = |value: f64| vec![Val::F64(value.to_bits())];
    assert_eq!(i32_result(call(&instance, "truncate", float(-3.9))), -3);
    assert_eq!(
        trap_message(call(&instance, "truncate", float(f64::NAN))),
        "float unrepresentable in integer range"
    );
    assert_eq!(
        trap_message(call(&instance, "truncate", float(2_147_483_648.0))),
        "float unrepresentable in integer range"
    );
    assert_eq!(
        i32_result(call(&instance, "truncate", float(-2_147_483_648.9))),
        i32::MIN
    );
    assert_eq!(
        i32_result(call(&instance, "saturate", float(1e20))),
        i32::MAX
    );
    assert_eq!(i32_result(call(&instance, "saturate", float(f64::NAN))), 0);
}

#[test]
fn float_operations_follow_webassembly_semantics() {
    let binary = |opcode: u8| body(&[], &[0x20, 0, 0x20, 1, opcode]);
    let unary = |opcode: u8| body(&[], &[0x20, 0, opcode]);
    let bytes = functions(
        &[
            func_type(&[F64, F64], &[F64]),
            func_type(&[F64], &[F64]),
            func_type(&[F32], &[F32]),
        ],
        &[
            (0, binary(0xA4)),
            (0, binary(0xA5)),
            (1, unary(0x9E)),
            (2, unary(0x90)),
            (0, binary(0xA6)),
            (2, unary(0x91)),
        ],
        &["min", "max", "nearest", "nearest32", "copysign", "sqrt32"],
    );
    let instance = instance(bytes, Vec::new());
    let f64_call = |name: &str, args: &[f64]| -> f64 {
        let values = args.iter().map(|value| Val::F64(value.to_bits())).collect();
        match call(&instance, name, values).expect("no trap").as_slice() {
            [Val::F64(bits)] => f64::from_bits(*bits),
            other => panic!("{other:?}"),
        }
    };
    assert!(f64_call("min", &[0.0, -0.0]).is_sign_negative());
    assert!(f64_call("max", &[-0.0, 0.0]).is_sign_positive());
    assert!(f64_call("min", &[1.0, f64::NAN]).is_nan());
    assert_eq!(f64_call("min", &[1.0, 2.0]), 1.0);
    assert_eq!(f64_call("nearest", &[2.5]), 2.0);
    assert_eq!(f64_call("nearest", &[3.5]), 4.0);
    assert_eq!(f64_call("nearest", &[-0.5]).to_bits(), (-0.0f64).to_bits());
    assert_eq!(f64_call("nearest", &[1e300]), 1e300);
    assert_eq!(f64_call("copysign", &[3.0, -1.0]), -3.0);
    let f32_call = |name: &str, value: f32| -> f32 {
        match call(&instance, name, vec![Val::F32(value.to_bits())])
            .expect("no trap")
            .as_slice()
        {
            [Val::F32(bits)] => f32::from_bits(*bits),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(f32_call("nearest32", 0.5), 0.0);
    assert_eq!(f32_call("nearest32", 1.5), 2.0);
    assert_eq!(f32_call("sqrt32", 2.0), core::f32::consts::SQRT_2);
}

#[test]
fn integer_operations_wrap_shift_and_count() {
    let binary = |opcode: u8| body(&[], &[0x20, 0, 0x20, 1, opcode]);
    let unary = |opcode: u8| body(&[], &[0x20, 0, opcode]);
    let bytes = functions(
        &[
            func_type(&[I32, I32], &[I32]),
            func_type(&[I32], &[I32]),
            func_type(&[I64, I64], &[I64]),
        ],
        &[
            (0, binary(0x74)),
            (0, binary(0x75)),
            (0, binary(0x77)),
            (1, unary(0x67)),
            (1, unary(0x69)),
            (1, unary(0xC0)),
            (2, binary(0x88)),
            (0, binary(0x49)),
        ],
        &[
            "shl", "shr_s", "rotl", "clz", "popcnt", "extend8", "shr_u64", "lt_u",
        ],
    );
    let instance = instance(bytes, Vec::new());
    let two = |name: &str, a, b| i32_result(call(&instance, name, vec![Val::I32(a), Val::I32(b)]));
    let one = |name: &str, a| i32_result(call(&instance, name, vec![Val::I32(a)]));
    assert_eq!(two("shl", 1, 33), 2);
    assert_eq!(two("shr_s", -8, 1), -4);
    assert_eq!(two("rotl", i32::MIN, 1), 1);
    assert_eq!(one("clz", 1), 31);
    assert_eq!(one("popcnt", -1), 32);
    assert_eq!(one("extend8", 0x80), -128);
    assert_eq!(two("lt_u", 1, -1), 1);
    assert_eq!(
        i64_result(call(&instance, "shr_u64", vec![Val::I64(-1), Val::I64(60)])),
        15
    );
}

#[test]
fn globals_tables_and_references_initialize_from_constant_expressions() {
    // Global 0: i32 = 40 + 2 (extended constants); global 1: mutable.
    let mut global0 = vec![I32, 0];
    global0.extend(i32_const(40));
    global0.extend(i32_const(2));
    global0.extend([0x6A, 0x0B]);
    let mut global1 = vec![I64, 1];
    global1.extend(i64_const(5));
    global1.push(0x0B);
    let get = body(&[], &[0x23, 0]);
    let bump = body(&[], &[0x23, 1, 0x42, 1, 0x7C, 0x24, 1, 0x23, 1]);
    // is_null(i) = ref.is_null(table.get(i)); size = table.size; grow.
    let is_null = body(&[], &[0x20, 0, 0x25, 0, 0xD1]);
    let size = body(&[], &[0xFC, 16, 0]);
    let grow = body(&[], &[0xD2, 0, 0x20, 0, 0xFC, 15, 0]);
    let mut elem = vec![0];
    elem.extend(i32_const(1));
    elem.push(0x0B);
    elem.extend(vector(&[u(0)]));
    let bytes = module(&[
        (
            1,
            vector(&[
                func_type(&[], &[I32]),
                func_type(&[], &[I64]),
                func_type(&[I32], &[I32]),
            ]),
        ),
        (3, vector(&[u(0), u(1), u(2), u(0), u(2)])),
        (4, vector(&[vec![FUNCREF, 0, 2]])),
        (6, vector(&[global0, global1])),
        (
            7,
            vector(&[
                export("get", 0, 0),
                export("bump", 0, 1),
                export("is_null", 0, 2),
                export("size", 0, 3),
                export("grow", 0, 4),
            ]),
        ),
        (9, vector(&[elem])),
        (10, vector(&[get, bump, is_null, size, grow])),
    ]);
    let instance = instance(bytes, Vec::new());
    assert_eq!(i32_result(call(&instance, "get", Vec::new())), 42);
    assert_eq!(i64_result(call(&instance, "bump", Vec::new())), 6);
    assert_eq!(i64_result(call(&instance, "bump", Vec::new())), 7);
    assert_eq!(i32_result(call(&instance, "is_null", vec![Val::I32(0)])), 1);
    assert_eq!(i32_result(call(&instance, "is_null", vec![Val::I32(1)])), 0);
    assert_eq!(
        trap_message(call(&instance, "is_null", vec![Val::I32(2)])),
        "table index is out of bounds"
    );
    assert_eq!(i32_result(call(&instance, "size", Vec::new())), 2);
    assert_eq!(i32_result(call(&instance, "grow", vec![Val::I32(3)])), 2);
    assert_eq!(i32_result(call(&instance, "is_null", vec![Val::I32(4)])), 0);
    assert_eq!(i32_result(call(&instance, "size", Vec::new())), 5);
}

#[test]
fn start_functions_run_at_instantiation() {
    let mut global = vec![I32, 1];
    global.extend(i32_const(0));
    global.push(0x0B);
    let start = body(&[], &[0x41, 9, 0x24, 0]);
    let get = body(&[], &[0x23, 0]);
    let bytes = module(&[
        (1, vector(&[func_type(&[], &[]), func_type(&[], &[I32])])),
        (3, vector(&[u(0), u(1)])),
        (6, vector(&[global])),
        (7, vector(&[export("get", 0, 1)])),
        (8, u(0)),
        (10, vector(&[start, get])),
    ]);
    let instance = instance(bytes, Vec::new());
    assert_eq!(i32_result(call(&instance, "get", Vec::new())), 9);
}

fn decode_error(bytes: Vec<u8>) -> CompileError {
    match decode::decode(bytes) {
        Err(error) => error,
        Ok(_) => panic!("expected a compile error"),
    }
}

#[test]
fn malformed_modules_fail_to_compile() {
    assert_eq!(
        decode_error(b"\x01\x02\x03\x04".to_vec()).describe(),
        "expected magic word 00 61 73 6d, found 01 02 03 04 @+0"
    );
    assert_eq!(
        decode_error(b"\0asm\x02\0\0\0".to_vec()).describe(),
        "expected version 01 00 00 00, found 02 00 00 00 @+4"
    );
    // A function whose body pops an empty stack.
    let bytes = functions(
        &[func_type(&[], &[I32])],
        &[(0, body(&[], &[0x6A]))],
        &["f"],
    );
    assert_eq!(
        decode_error(bytes).message,
        "not enough arguments on the stack"
    );
    // A function missing its result.
    let bytes = functions(&[func_type(&[], &[I32])], &[(0, body(&[], &[]))], &["f"]);
    assert_eq!(
        decode_error(bytes).message,
        "expected a different number of values at the end of the block"
    );
    // SIMD is not supported.
    let bytes = functions(
        &[func_type(&[], &[])],
        &[(0, body(&[], &[0xFD, 0]))],
        &["f"],
    );
    assert_eq!(decode_error(bytes).message, "Wasm SIMD unsupported");
    // Code after the function's end.
    let mut bytes = functions(&[func_type(&[], &[])], &[(0, body(&[], &[]))], &["f"]);
    bytes.extend([10, 1, 0]);
    assert!(
        decode_error(bytes)
            .message
            .starts_with("unexpected section")
    );
    // Overlong LEB128.
    assert_eq!(
        decode_error(module(&[(1, vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x00])])).message,
        "length overflow while decoding integer"
    );
}

#[test]
fn custom_sections_and_unreachable_code_are_accepted() {
    // Code after `return` and `br` is skipped, including nested blocks.
    let mut code = i32_const(1);
    code.extend([0x0F, 0x02, 0x40, 0x6A, 0x1A, 0x0B, 0x6A]);
    let mut bytes = functions(&[func_type(&[], &[I32])], &[(0, body(&[], &code))], &["f"]);
    let mut custom = name("note");
    custom.extend_from_slice(b"hello");
    bytes.push(0);
    bytes.extend(u(custom.len() as u64));
    bytes.extend(custom);
    let module = decode::decode(bytes).expect("a valid module");
    assert_eq!(module.customs.len(), 1);
    assert_eq!(module.customs[0].0, "note");
    assert_eq!(&module.bytes[module.customs[0].1.clone()], b"hello");
    let instance = match exec::instantiate(&mut host(), Rc::new(module), Vec::new()) {
        Ok(instance) => instance,
        Err(error) => panic!("{error:?}"),
    };
    assert_eq!(i32_result(call(&instance, "f", Vec::new())), 1);
}

#[test]
fn signed_leb128_immediates_decode() {
    for value in [
        0i64,
        -1,
        63,
        -64,
        64,
        -65,
        i64::from(i32::MAX),
        i64::from(i32::MIN),
    ] {
        let code = i32_const(value as i32);
        let bytes = functions(&[func_type(&[], &[I32])], &[(0, body(&[], &code))], &["f"]);
        let instance = instance(bytes, Vec::new());
        assert_eq!(i32_result(call(&instance, "f", Vec::new())), value as i32);
    }
    for value in [i64::MAX, i64::MIN, -1, 1 << 40] {
        let code = i64_const(value);
        let bytes = functions(&[func_type(&[], &[I64])], &[(0, body(&[], &code))], &["f"]);
        let instance = instance(bytes, Vec::new());
        assert_eq!(i64_result(call(&instance, "f", Vec::new())), value);
    }
    let _ = (F32, F64);
}
