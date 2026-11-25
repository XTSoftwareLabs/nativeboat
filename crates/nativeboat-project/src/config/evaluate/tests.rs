use super::*;
use oxc::{allocator::Allocator, parser::Parser, span::SourceType};

fn evaluate(source: &str) -> Data {
    let mut bindings = bindings(Path::new(if cfg!(windows) {
        "C:/projects/config.ts"
    } else {
        "/projects/config.ts"
    }));
    bindings.insert("path".to_owned(), module("path").unwrap());
    bindings.insert("url".to_owned(), module("url").unwrap());
    evaluate_with(source, &bindings)
}

fn evaluate_with(source: &str, bindings: &BTreeMap<String, Data>) -> Data {
    let allocator = Allocator::default();
    let source = format!("const result = {source}");
    let parsed = Parser::new(&allocator, &source, SourceType::ts()).parse();
    let Statement::VariableDeclaration(declaration) = &parsed.program.body[0] else {
        panic!("expected variable")
    };
    value(declaration.declarations[0].init.as_ref().unwrap(), bindings)
}

#[test]
fn literals_templates_and_json_stringification_keep_javascript_semantics() {
    for (source, expected) in [
        ("JSON.stringify('1.2.3')", r#""\"1.2.3\"""#),
        (
            "`v${1}:${true}:${null}:${1e21}:${1e-7}`",
            r#""v1:true:null:1e+21:1e-7""#,
        ),
        (
            "JSON.stringify({z:1,a:2,'10':3,'2':4,'01':5,'4294967295':6})",
            r#""{\"2\":4,\"10\":3,\"z\":1,\"a\":2,\"01\":5,\"4294967295\":6}""#,
        ),
        (
            "JSON.stringify([null,true,1e20,'escaped\\n'])",
            r#""[null,true,100000000000000000000,\"escaped\\n\"]""#,
        ),
        (
            "JSON.stringify(JSON.stringify({z:1,a:2}))",
            r#""\"{\\\"z\\\":1,\\\"a\\\":2}\"""#,
        ),
        ("({flag:true} as const)['flag']", "true"),
        (
            "JSON.stringify({['__proto__']:1})",
            r#""{\"__proto__\":1}""#,
        ),
    ] {
        assert_eq!(
            evaluate(source).stringify().as_deref(),
            Some(expected),
            "{source}"
        );
    }
    assert_eq!(
        evaluate("{a:1,b:[null,true]}").json(true),
        Some(serde_json::json!({"a":1.0,"b":[null,true]}))
    );
    assert_eq!(
        evaluate("{a:unknown,b:[unknown]}").json(false),
        Some(serde_json::json!({"a":null,"b":[null]}))
    );
    assert!(evaluate("{a:unknown}").json(true).is_none());
    assert!(evaluate("[unknown]").json(true).is_none());
    assert!(
        evaluate("({}) satisfies Config")
            .json(true)
            .unwrap()
            .is_object()
    );
}

#[test]
fn unknown_and_invalid_expressions_are_preserved_without_execution() {
    for source in [
        "unknown",
        "1 + 2",
        "-1",
        "1e999",
        "unknown.name",
        "({a:1})[unknown]",
        "({a:1})[42]",
        "`x${unknown}`",
        "`x${{}}`",
        "`\\xZZ`",
        "{[unknown]:1}",
        "JSON.stringify(unknown)",
        "JSON.stringify({...unknown})",
        "JSON.stringify([unknown])",
        "JSON.stringify(...unknown)",
        "JSON.stringify({},null,2)",
        "JSON.stringify()",
        "run()",
        "path.join(...unknown)",
        "path.join(42)",
        "Object['assign']({},{})",
        "JSON.stringify({__proto__:{a:1}})",
    ] {
        let data = evaluate(source);
        assert!(data.stringify().is_none(), "{source}: {data:?}");
    }
    assert!(Data::Array(Vec::new()).property("missing").is_none());
}

use std::path::Path;

#[test]
fn static_path_and_url_helpers_preserve_roots_and_url_encoding() {
    let config = if cfg!(windows) {
        "C:/project space/#项目/config.ts"
    } else {
        "/project space/#项目/config.ts"
    };
    let mut environment = bindings(Path::new(config));
    environment.insert("path".to_owned(), module("node:path").unwrap());
    environment.insert("url".to_owned(), module("node:url").unwrap());
    let evaluate = |source| evaluate_with(source, &environment).json(true).unwrap();
    let root = if cfg!(windows) { "C:/" } else { "/" };
    for (source, expected) in [
        (
            "path.resolve('child', '../main.ts')",
            config.replace("config.ts", "main.ts"),
        ),
        ("path.resolve()", config.replace("/config.ts", "")),
        ("path.join('', 'a', '..')", ".".to_owned()),
        ("path.join()", ".".to_owned()),
        ("path.join('./a', 'b')", "a/b".to_owned()),
        ("path.join('../a', '../../b')", "../../b".to_owned()),
        (
            "path.resolve(__dirname, '../../../..', 'main.ts')",
            format!("{root}main.ts"),
        ),
        ("path.dirname('file.ts')", ".".to_owned()),
        ("path.dirname('')", ".".to_owned()),
        (
            "path.dirname(__dirname)",
            config.replace("/#项目/config.ts", ""),
        ),
        ("path.dirname(path.resolve('../../../..'))", root.to_owned()),
        (
            "path.join(url.fileURLToPath(import.meta.url))",
            config.to_owned(),
        ),
        (
            "path.join(url.fileURLToPath(new URL('./nested/../main.ts', import.meta.url)))",
            config.replace("config.ts", "main.ts"),
        ),
        (
            "new URL('../file%20name.ts', new URL('file:///dir/sub/config.ts')).href",
            "file:///dir/file%20name.ts".to_owned(),
        ),
        (
            "JSON.stringify(new URL('https://example.org/a b'))",
            "\"https://example.org/a%20b\"".to_owned(),
        ),
        (
            "`${new URL('https://example.org')}`",
            "https://example.org/".to_owned(),
        ),
        ("require('path').join('a', 'b')", "a/b".to_owned()),
    ] {
        assert_eq!(evaluate(source), Value::String(expected), "{source}");
    }
    let meta = evaluate("import.meta.url");
    let meta = meta.as_str().unwrap();
    assert!(meta.contains("project%20space/%23%E9%A1%B9%E7%9B%AE/config.ts"));
    assert!(
        evaluate_with("JSON.stringify(path.resolve)", &environment)
            .json(true)
            .is_none()
    );
    assert_eq!(
        evaluate_with("path.resolve", &environment).json(false),
        Some(Value::Null)
    );
}

#[test]
fn invalid_url_paths_and_untrusted_functions_remain_unknown() {
    for source in [
        "url.fileURLToPath(42)",
        "url.fileURLToPath('relative')",
        "url.fileURLToPath('https://example.org/file')",
        "url.fileURLToPath('file:///a%2Fb')",
        "url.fileURLToPath()",
        "url.fileURLToPath(...unknown)",
        "new URL()",
        "new URL('a', 'relative')",
        "new URL('a', 42)",
        "new URL(42)",
        "new URL('relative')",
        "new URL('https://example.org', {}, true)",
        "new URL('https://example.org').missing",
        "URL('https://example.org')",
        "new Unknown('file:///a')",
        "other.resolve('/a')",
        "({resolve: unknown}).resolve('/a')",
        "require('unknown')",
        "require(42)",
        "path.dirname(42)",
    ] {
        assert!(evaluate(source).json(true).is_none(), "{source}");
    }
    #[cfg(windows)]
    assert!(
        evaluate("url.fileURLToPath('file:///C:/a%5Cb')")
            .json(true)
            .is_none()
    );
    let mut environment = bindings(Path::new(""));
    assert!(
        evaluate_with("import.meta.url", &environment)
            .json(true)
            .is_none()
    );
    environment.insert("resolve".to_owned(), Data::Function(Function::Resolve));
    environment.remove("\0cwd");
    assert!(
        evaluate_with("resolve('a')", &environment)
            .json(true)
            .is_none()
    );
}
