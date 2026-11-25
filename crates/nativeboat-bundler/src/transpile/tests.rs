use super::*;

fn run(source: &str, file: &str) -> Result<String, Vec<String>> {
    transpile(source, Path::new(file), &TranspileOptions::default())
}

#[test]
fn source_types_follow_extensions() {
    let kind = |file: &str| source_type(Path::new(file), true).unwrap();
    assert!(kind("a.js").is_jsx());
    assert!(!source_type(Path::new("a.js"), false).unwrap().is_jsx());
    assert!(kind("a.JSX").is_jsx());
    assert!(kind("a.mjs").is_module());
    assert!(kind("a.cjs").is_commonjs());
    assert!(kind("a.ts").is_typescript());
    assert!(kind("a.mts").is_typescript());
    assert!(kind("a.cts").is_typescript());
    assert!(kind("a.tsx").is_jsx());
    assert_eq!(
        source_type(Path::new("a.vue"), true).unwrap_err(),
        "a.vue: unsupported source file type"
    );
    assert!(source_type(Path::new("Makefile"), true).is_err());
}

#[test]
fn typescript_is_stripped() {
    let code = run(
        "import type { T } from './t';\ninterface I { a: number }\nconst x: number = 1 as number;\nexport { x };",
        "a.ts",
    )
    .unwrap();
    assert!(!code.contains("interface"));
    assert!(!code.contains("./t"));
    assert!(code.contains("const x = 1;"));
}

#[test]
fn jsx_uses_the_automatic_runtime() {
    let code = run(
        "export const App = () => <div id=\"a\">{1}</div>;",
        "App.tsx",
    )
    .unwrap();
    assert!(code.contains("from \"react/jsx-runtime\""));
    assert!(code.contains("_jsx(\"div\""));
    let options = TranspileOptions {
        jsx_import_source: "preact".to_owned(),
        ..TranspileOptions::default()
    };
    let code = transpile("const a = <b/>;", Path::new("a.jsx"), &options).unwrap();
    assert!(code.contains("preact/jsx-runtime"));
}

#[test]
fn defines_are_replaced() {
    let code = run(
        "if (process.env.NODE_ENV !== 'production') { debug(); }",
        "a.js",
    )
    .unwrap();
    assert!(code.contains("\"production\" !== \"production\""));
    let options = TranspileOptions {
        defines: Vec::new(),
        ..TranspileOptions::default()
    };
    let code = transpile("process.env.NODE_ENV;", Path::new("a.cjs"), &options).unwrap();
    assert!(code.contains("process.env.NODE_ENV"));
    let options = TranspileOptions {
        defines: vec![("1bad key".to_owned(), "1".to_owned())],
        ..TranspileOptions::default()
    };
    let errors = transpile("x;", Path::new("a.js"), &options).unwrap_err();
    assert!(errors[0].starts_with("a.js: "));
}

#[test]
fn errors_name_the_file() {
    let errors = run("const = ;", "broken.ts").unwrap_err();
    assert!(errors[0].starts_with("broken.ts: "));
    assert!(run("x", "a.txt").is_err());
    let errors = run("const a = <div key />;", "key.jsx").unwrap_err();
    assert!(errors[0].starts_with("key.jsx: "), "{errors:?}");
}

#[test]
fn experimental_decorators_use_runtime_helpers() {
    let source = "function log(target: any, key?: any) {}\n\
        @log class Service { @log run(@log value: string): string { return value; } }\n";
    let options = TranspileOptions {
        legacy_decorators: true,
        decorator_metadata: true,
        ..TranspileOptions::default()
    };
    let code = transpile(source, Path::new("service.ts"), &options).unwrap();
    assert!(code.contains("babelHelpers.decorate("), "{code}");
    assert!(code.contains("babelHelpers.decorateParam("), "{code}");
    assert!(
        code.contains("babelHelpers.decorateMetadata(\"design:type\""),
        "{code}"
    );
    assert!(
        !code.contains("@log") && !code.contains("import "),
        "{code}"
    );
}
