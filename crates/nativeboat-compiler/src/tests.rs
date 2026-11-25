use super::*;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn module(source: &str) -> NativeModule {
    compile(source, Path::new("behavior.ts")).unwrap()
}

fn symbol(module: &NativeModule, name: &str) -> String {
    module.export_name(name).unwrap()
}

fn execute(module: &NativeModule, main: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/task114-native-tests");
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = root.join(format!("{}-{id}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("generated.rs");
    let executable = directory.join(format!("generated{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        &source,
        format!(
            "{}\nfn main() {{ {main}\nprintln!(\"native behavior passed\"); }}\n",
            module.rust_source()
        ),
    )
    .unwrap();
    let compiled = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-name",
            "nativeboat_generated",
            "-C",
            "opt-level=1",
            "-C",
            "debuginfo=0",
            "-D",
            "warnings",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new(&executable).output().unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout).trim(),
        "native behavior passed"
    );
    // Windows can briefly retain a mapped executable after it exits.
    for _ in 0..20 {
        if fs::remove_dir_all(&directory).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn generated_native_code_executes_scalar_semantics_without_an_engine() {
    let source = r#"
        const base: number = 2;
        export const flag: boolean = true;
        function hidden(x: number): number { return x; }
        export const add = (x: number, y: number): number => x + y;
        export function subtract(x: number, y: number): number { return x - y; }
        export const multiply = (x: number, y: number) => x * y;
        export const divide = (x: number, y: number) => x / y;
        export const remainder = (x: number, y: number) => x % y;
        export const less = (x: number, y: number) => x < y;
        export const lessEqual = (x: number, y: number) => x <= y;
        export const greater = (x: number, y: number) => x > y;
        export const greaterEqual = (x: number, y: number) => x >= y;
        export const equal = (x: number, y: number) => x === y;
        export const different = (x: number, y: number) => x !== y;
        export const choose = (b: boolean, x: number, y: number) => b ? x : y;
        export const and = (a: boolean, b: boolean) => a && b;
        export const or = (a: boolean, b: boolean) => a || b;
        export const not = (a: boolean) => !a;
        export const negate = (x: number) => -x;
        export const plus = (x: number) => +x;
        export const abs = (x: number) => Math.abs(x);
        export const parens = (x: number) => (x + base) * base;
        export const infinity = () => 1e309;
        export const isOdd = (n: number) => Math.abs(n) % 2 === 1;
        export const isEven = (n: number) => Math.abs(n) % 2 === 0;
        export const blocked = (base: boolean): boolean => { 'use strict'; return base === flag; };
    "#;
    let module = module(source);
    assert_eq!(module.clone(), module);
    assert!(format!("{module:?}").contains("NativeModule"));
    assert_eq!(module.export_name("missing"), None);
    assert_eq!(module.export_name("hidden"), None);
    let rust = module.rust_source();
    for forbidden in [
        "boa",
        "eval(",
        "document.",
        "export const",
        "require(",
        "nativeboat_web",
    ] {
        assert!(!rust.contains(forbidden), "{forbidden}");
    }
    let mut main = String::new();
    for (name, arguments, expected) in [
        ("add", "2.0, 3.0", "5.0"),
        ("subtract", "2.0, 3.0", "-1.0"),
        ("multiply", "2.0, 3.0", "6.0"),
        ("divide", "6.0, 3.0", "2.0"),
        ("remainder", "-5.0, 2.0", "-1.0"),
        ("less", "2.0, 3.0", "true"),
        ("lessEqual", "3.0, 3.0", "true"),
        ("greater", "2.0, 3.0", "false"),
        ("greaterEqual", "3.0, 3.0", "true"),
        ("equal", "0.0, -0.0", "true"),
        ("different", "2.0, 3.0", "true"),
        ("choose", "true, 2.0, 3.0", "2.0"),
        ("choose", "false, 2.0, 3.0", "3.0"),
        ("and", "true, false", "false"),
        ("or", "false, true", "true"),
        ("not", "false", "true"),
        ("negate", "2.0", "-2.0"),
        ("plus", "2.0", "2.0"),
        ("abs", "-2.0", "2.0"),
        ("parens", "3.0", "10.0"),
        ("infinity", "", "f64::INFINITY"),
        ("blocked", "false", "false"),
        ("isOdd", "-3.0", "true"),
        ("isEven", "-4.0", "true"),
        ("isOdd", "f64::NAN", "false"),
        ("isEven", "f64::INFINITY", "false"),
        ("equal", "f64::NAN, f64::NAN", "false"),
    ] {
        main.push_str(&format!(
            "assert_eq!({}({arguments}), {expected});\n",
            symbol(&module, name)
        ));
    }
    for (name, arguments, assertion) in [
        ("negate", "0.0", ".is_sign_negative()"),
        ("abs", "-0.0", ".is_sign_positive()"),
        ("abs", "f64::NAN", ".is_nan()"),
        ("divide", "0.0, 0.0", ".is_nan()"),
        ("remainder", "-4.0, 2.0", ".is_sign_negative()"),
    ] {
        main.push_str(&format!(
            "assert!({}({arguments}){assertion});\n",
            symbol(&module, name)
        ));
    }
    execute(&module, &main);
}

#[test]
fn every_unsupported_operation_is_diagnosed_without_a_fallback() {
    for (source, message) in [
        ("import x from 'electron';", "statement"),
        ("export interface X {}", "declaration"),
        ("let x = 1", "initialized const"),
        ("declare const x: number;", "initialized const"),
        ("const { x } = {}", "destructured declarations"),
        ("const x: number;", "initializer"),
        ("const x = 1 + 2;", "module constants"),
        ("const x: boolean = 1;", "declared type"),
        ("const x: string = 1;", "scalar types"),
        (
            "const f: (x: number) => number = (x: number) => x;",
            "binding annotations",
        ),
        ("async function f() { return 1; }", "async, generator"),
        ("function* f() { return 1; }", "async, generator"),
        ("declare function f(): number;", "async, generator"),
        ("function f<T>(x: number) { return x; }", "async, generator"),
        ("function f(this: object) { return 1; }", "async, generator"),
        ("function f(): number;", "require a body"),
        ("const f = async () => 1;", "async or generic"),
        ("const f = <T>(x: number) => x;", "async or generic"),
        ("const f = (...x: number[]) => 1;", "rest parameters"),
        (
            "const f = ({x}: {x: number}) => x;",
            "destructured parameters",
        ),
        ("const f = (x?: number) => 1;", "optional, defaulted"),
        ("const f = (x: number = 1) => x;", "optional, defaulted"),
        ("const f = x => x;", "require a number"),
        ("const f = (x: string) => x;", "scalar types"),
        ("function f() { return; }", "return a value"),
        ("function f() {}", "every path"),
        ("const f = (): boolean => 1;", "declared type"),
        ("const f = () => missing;", "scalar binding"),
        ("const f = () => 'hello';", "expression"),
        ("const f = (x: number) => typeof x;", "unary operator"),
        ("const f = (x: boolean) => +x;", "different scalar"),
        ("const f = (x: boolean) => -x;", "different scalar"),
        ("const f = (x: number) => !x;", "different scalar"),
        ("const f = (x: number) => x == 1;", "binary operator"),
        ("const f = (x: boolean) => x + 1;", "different scalar"),
        ("const f = (x: number) => x + true;", "different scalar"),
        ("const f = (x: number) => x === true;", "different scalar"),
        ("const f = (x: number) => x ?? 1;", "nullish expressions"),
        ("const f = (x: number) => x && true;", "different scalar"),
        ("const f = (x: boolean) => x || 1;", "different scalar"),
        ("const f = (x: number) => x ? 1 : 2;", "different scalar"),
        (
            "const f = (x: boolean) => x ? 1 : false;",
            "different scalar",
        ),
        ("const f = (x: number) => unknown(x);", "call target"),
        ("const f = (x: number) => ({}).abs(x);", "intrinsic object"),
        (
            "const f = (x: number) => Math.sqrt(x);",
            "unshadowed Math.abs",
        ),
        (
            "const f = (Math: number) => Math.abs(1);",
            "unshadowed Math.abs",
        ),
        (
            "const Math = 2; const f = () => Math.abs(1);",
            "unshadowed Math.abs",
        ),
        ("const f = () => Math.abs();", "exactly one numeric"),
        ("const f = () => Math.abs(1, 2);", "exactly one numeric"),
        (
            "const f = (x: number) => Math.abs(...x);",
            "spread arguments",
        ),
        ("const f = (x: boolean) => Math.abs(x);", "different scalar"),
        ("const f = () => eval('1');", "call target"),
        (
            "const f = () => document.createElement('p');",
            "unshadowed Math.abs",
        ),
    ] {
        let errors = compile(source, Path::new("unsupported.ts")).unwrap_err();
        assert!(errors[0].message.contains(message), "{source}: {errors:?}");
        assert_eq!(errors[0].path, Path::new("unsupported.ts"));
        assert!(errors[0].line >= 1 && errors[0].column >= 1);
        assert!(format!("{}", errors[0]).starts_with("unsupported.ts:"));
    }
}

#[test]
fn generated_calls_locals_and_branches_execute_natively() {
    let module = module(
        r#"
        const factor = 2;
        export function run(x: number, keep: boolean): number {
            ;
            const seed = helper(x);
            let value: number = seed, extra: number = 0;
            { const value = 7; extra = value; }
            if (keep) { value = value + extra; }
            else if (x < 0) { return negate(value); }
            value = forward(value);
            value;
            return value;
        }
        function helper(x: number) { x = x + 1; return x; }
        const forward = (x: number) => { return (scale)(x); };
        export const scale = (x: number) => x * factor;
        function negate(x: number) { return -x; }
        function boolean(x: boolean) { if (x) return true; else return false; }
        export const shortOr = (x: boolean) => x || boolean(false);
        export const shortAnd = (x: boolean) => x && boolean(true);
        export const nested = (x: boolean) => { { if (x) return true; } return false; };
        export const localFlag = (x: boolean) => { let flag = x; flag = !flag; return flag; };
    "#,
    );
    assert_eq!(module.clone(), module);
    assert!(format!("{module:?}").contains("Call"));
    let mut main = String::new();
    for (name, arguments, expected) in [
        ("run", "2.0, true", "20.0"),
        ("run", "2.0, false", "6.0"),
        ("run", "-3.0, false", "2.0"),
        ("shortOr", "true", "true"),
        ("shortAnd", "false", "false"),
        ("shortOr", "false", "false"),
        ("shortAnd", "true", "true"),
        ("nested", "true", "true"),
        ("nested", "false", "false"),
        ("localFlag", "false", "true"),
    ] {
        main.push_str(&format!(
            "assert_eq!({}({arguments}), {expected});\n",
            symbol(&module, name)
        ));
    }
    execute(&module, &main);
}

#[test]
fn native_extrema_preserve_nan_infinities_and_signed_zero() {
    let module = module(
        r#"
        export const min = (x: number, y: number, z: number) => Math.min(x, y, z);
        export const max = (x: number, y: number, z: number) => Math.max(x, y, z);
        export const noMin = () => Math.min();
        export const noMax = () => Math.max();
        export const one = (x: number) => Math.min(x);
        export const clamp = (x: number, low: number, high: number) => Math.max(low, Math.min(high, x));
    "#,
    );
    let mut main = format!(
        "assert_eq!({}(), f64::INFINITY); assert_eq!({}(), f64::NEG_INFINITY); assert_eq!({}(5.0), 5.0);\n",
        symbol(&module, "noMin"),
        symbol(&module, "noMax"),
        symbol(&module, "one")
    );
    for (name, arguments, expected) in [
        ("min", "3.0, -2.0, 1.0", "-2.0"),
        ("max", "3.0, -2.0, 1.0", "3.0"),
        (
            "min",
            "f64::INFINITY, 2.0, f64::NEG_INFINITY",
            "f64::NEG_INFINITY",
        ),
        (
            "max",
            "f64::NEG_INFINITY, 2.0, f64::INFINITY",
            "f64::INFINITY",
        ),
        ("clamp", "-1.0, 0.0, 10.0", "0.0"),
        ("clamp", "5.0, 0.0, 10.0", "5.0"),
        ("clamp", "11.0, 0.0, 10.0", "10.0"),
    ] {
        main.push_str(&format!(
            "assert_eq!({}({arguments}), {expected});\n",
            symbol(&module, name)
        ));
    }
    for name in ["min", "max"] {
        for arguments in [
            "f64::NAN, 0.0, 1.0",
            "0.0, f64::NAN, -0.0",
            "1.0, -1.0, f64::NAN",
        ] {
            main.push_str(&format!(
                "assert!({}({arguments}).is_nan());\n",
                symbol(&module, name)
            ));
        }
        for arguments in ["0.0, -0.0, 0.0", "-0.0, 0.0, -0.0"] {
            let sign = if name == "min" {
                "negative"
            } else {
                "positive"
            };
            main.push_str(&format!(
                "assert!({}({arguments}).is_sign_{sign}());\n",
                symbol(&module, name)
            ));
        }
    }
    main.push_str(&format!(
        "assert!({}(-0.0).is_sign_negative()); assert!({}(f64::NAN).is_nan());\n",
        symbol(&module, "one"),
        symbol(&module, "one")
    ));
    execute(&module, &main);
}

#[test]
fn native_control_flow_and_call_errors_keep_source_diagnostics() {
    for (source, message) in [
        ("function f() { if (true) return 1; }", "every path"),
        (
            "function f() { if (true) return 1; return false; }",
            "different scalar types",
        ),
        (
            "function f() { if (true) return 1; else return false; }",
            "different scalar types",
        ),
        (
            "function f() { return 1; return 2; }",
            "unreachable statements",
        ),
        (
            "function f() { if (1) return 1; else return 2; }",
            "different scalar",
        ),
        (
            "function f() { var x = 1; return x; }",
            "initialized const or let",
        ),
        (
            "function f() { const {x} = 1; return 1; }",
            "destructured locals",
        ),
        ("function f() { let x: number; return 1; }", "initializer"),
        (
            "function f() { let x: boolean = 1; return x; }",
            "declared type",
        ),
        (
            "function f() { let x = 1; x += 1; return x; }",
            "compound assignments",
        ),
        (
            "function f() { let x = 1; x.a = 2; return x; }",
            "scalar binding assignments",
        ),
        (
            "function f() { missing = 1; return 1; }",
            "not a compiled scalar",
        ),
        (
            "function f() { x = 1; let x = 2; return x; }",
            "not initialized",
        ),
        (
            "function f() { const x = 1; x = 2; return x; }",
            "const binding",
        ),
        (
            "function f() { let x = 1; x = false; return x; }",
            "different scalar",
        ),
        ("function f() { return x; let x = 2; }", "scalar binding"),
        ("function f() { while (true) {} return 1; }", "statement"),
        ("const f = () => f();", "recursive native calls"),
        (
            "function f(): number { return f(); }",
            "stack-limit and exception",
        ),
        (
            "function f(): number { return g(); } function g(): number { return f(); }",
            "recursive native calls",
        ),
        (
            "function f(): boolean { return true; } const g = () => f() + 1;",
            "different scalar",
        ),
        (
            "function f(x: number) { return x; } const g = () => f();",
            "argument count",
        ),
        (
            "function f(x: number) { return x; } const g = () => f(true);",
            "different scalar",
        ),
        (
            "function f(x: number) { return x; } const g = () => f(...1);",
            "spread arguments",
        ),
        (
            "function f(x: number) { return x; } const g = () => f<number>(1);",
            "optional or generic",
        ),
        (
            "function f(x: number) { return x; } const g = () => f?.(1);",
            "optional calls",
        ),
        ("const f = () => Math.abs?.(1);", "optional calls"),
        ("const f = () => Math['abs'](1);", "native intrinsic"),
        ("const f = () => (() => 1)();", "native intrinsic"),
        (
            "function f() { return 1; } const g = (f: number) => f();",
            "native function",
        ),
        ("const f = () => Math.min(...1);", "spread arguments"),
        ("const f = () => Math.max(true);", "different scalar"),
    ] {
        let diagnostic = &compile(source, Path::new("control.ts")).unwrap_err()[0];
        assert!(
            diagnostic.message.contains(message),
            "{source}: {diagnostic}"
        );
        assert_eq!(diagnostic.path, Path::new("control.ts"));
        assert!(diagnostic.span.start < diagnostic.span.end);
        assert!(diagnostic.line >= 1 && diagnostic.column >= 1);
    }
}

#[test]
fn parser_semantic_and_unicode_locations_remain_original_source_locations() {
    assert!(compile("const f = (", Path::new("bad.ts")).is_err());
    let errors = compile("const x = 1; const x = 2;", Path::new("duplicate.ts")).unwrap_err();
    assert!(errors[0].message.contains("already"));
    assert!(compile("", Path::new("bad.invalid")).is_err());
    assert!(
        compile("", Path::new("empty.ts"))
            .unwrap()
            .rust_source()
            .contains("Generated native")
    );
    assert!(
        compile(
            "export function f() { return true; }",
            Path::new("plain.js")
        )
        .is_ok()
    );
    assert!(compile("export const f = () => 1;", Path::new("plain.mjs")).is_ok());
    let errors = compile(
        "// 原始来源\nexport const f = (数字: number) => 数字 + unknown;",
        Path::new("原始.ts"),
    )
    .unwrap_err();
    assert_eq!(errors[0].line, 2);
    assert_eq!(errors[0].column, 39);
    assert_eq!(
        &"// 原始来源\nexport const f = (数字: number) => 数字 + unknown;"
            [errors[0].span.start as usize..errors[0].span.end as usize],
        "unknown"
    );
    let module = module(
        "const value = 2; export const first = (value: number) => value + 1; export const second = (value: boolean) => !value; export const third = () => value;",
    );
    let main = format!(
        "assert_eq!({}(5.0), 6.0); assert!({}(false)); assert_eq!({}(), 2.0);",
        symbol(&module, "first"),
        symbol(&module, "second"),
        symbol(&module, "third")
    );
    execute(&module, &main);
}

#[test]
#[ignore = "requires NATIVEBOAT_MARKTEXT_PROJECT with unchanged MarkText sources"]
fn unchanged_marktext_numeric_helpers_compile_and_execute_natively() {
    let project = PathBuf::from(
        std::env::var_os("NATIVEBOAT_MARKTEXT_PROJECT").expect("MarkText source root"),
    );
    let path = project.join("packages/muya/src/utils/index.ts");
    let source = fs::read_to_string(&path).unwrap();
    let helpers = source
        .lines()
        .filter(|line| {
            line.starts_with("export const isOdd =") || line.starts_with("export const isEven =")
        })
        .collect::<Vec<_>>();
    assert_eq!(helpers.len(), 2);
    let module = compile(&helpers.join("\n"), &path).unwrap();
    execute(
        &module,
        &format!(
            "assert!({}(-3.0)); assert!({}(-4.0)); assert!(!{}(f64::NAN));",
            symbol(&module, "isOdd"),
            symbol(&module, "isEven"),
            symbol(&module, "isEven")
        ),
    );
}
