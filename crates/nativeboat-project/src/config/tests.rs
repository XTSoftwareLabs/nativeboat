use super::*;
use crate::tests::fixture::Fixture;

#[test]
fn jsonc_is_stripped() {
    let text = "{\n  // comment\n  \"a\": \"//not a comment\", /* block */ \"b\": [1, 2,],\n  \"c\": \"q\\\"uote\",\n}";
    let value: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
    assert_eq!(value["a"], "//not a comment");
    assert_eq!(value["b"], serde_json::json!([1, 2]));
    assert_eq!(value["c"], "q\"uote");
    assert_eq!(strip_jsonc("[1] // tail"), "[1] ");
    assert_eq!(strip_jsonc("/* open"), "");
    // Trailing commas before comments.
    let commented = "{\"a\": 1, // last\n /* really */ }";
    let value: Value = serde_json::from_str(&strip_jsonc(commented)).unwrap();
    assert_eq!(value["a"], 1);
    assert_eq!(strip_jsonc("[1, /* open"), "[1, ");
    assert_eq!(strip_jsonc("[1,"), "[1,");
    assert_eq!(strip_jsonc("\"a\\"), "\"a\\");
}

#[test]
fn tsconfig_paths() {
    let fixture = Fixture::new(&[
        (
            "tsconfig.json",
            "{ \"compilerOptions\": { \"baseUrl\": \".\", \"paths\": { \"@/*\": [\"./src/*\"], \"~lib\": [\"lib/index.ts\"], \"bad\": [] } } }",
        ),
        (
            "tsconfig.app.json",
            "{ // app\n \"compilerOptions\": { \"baseUrl\": \"src\", \"paths\": { \"@/*\": [\"other/*\"], \"#ui/*\": [\"ui/*\"] } } }",
        ),
        ("tsconfig.web.json", "{ \"compilerOptions\": {} }"),
        ("tsconfig.node.json", "not json"),
    ]);
    let aliases = tsconfig_aliases(fixture.root());
    let pairs: Vec<(&str, &str)> = aliases
        .iter()
        .map(|alias| (alias.pattern.as_str(), alias.target.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("@/*", "src/*"),
            ("~lib", "lib/index.ts"),
            ("#ui/*", "src/ui/*")
        ]
    );
    assert!(tsconfig_aliases(Fixture::new(&[]).root()).is_empty());
}

#[test]
fn vite_alias_forms() {
    let fixture = Fixture::new(&[
        (
            "vite.config.ts",
            "import { defineConfig } from 'vite';\nimport path from 'path';\n\
             export default defineConfig({ resolve: { alias: {\n\
               '@': path.resolve(__dirname, './src'),\n\
               components: fileURLToPath(new URL('./src/components/', import.meta.url)),\n\
               'lit': './lib/lit',\n\
               [computed]: './x',\n\
               dynamic: someFunction(),\n\
               ident: someVariable,\n\
             } } });",
        ),
        (
            "electron.vite.config.ts",
            "export default { renderer: { resolve: { alias: [\n\
               { find: '@renderer', replacement: resolve('src/renderer/src') },\n\
               { find: 'incomplete' },\n\
               { find: 'x', replacement: './y', extra: 1, ...rest },\n\
               'not-an-object',\n\
             ] } }, other: { alias: 42 } };",
        ),
    ]);
    let pairs = |file: &str| -> Vec<(String, String)> {
        vite_aliases(&fixture.path(file), fixture.root())
            .into_iter()
            .map(|alias| (alias.pattern, alias.target))
            .collect()
    };
    assert_eq!(
        pairs("vite.config.ts"),
        vec![
            ("@".to_owned(), "src".to_owned()),
            ("@/*".to_owned(), "src/*".to_owned()),
            ("components".to_owned(), "src/components".to_owned()),
            ("components/*".to_owned(), "src/components/*".to_owned()),
            ("lit".to_owned(), "lib/lit".to_owned()),
            ("lit/*".to_owned(), "lib/lit/*".to_owned()),
        ]
    );
    assert_eq!(
        pairs("electron.vite.config.ts"),
        vec![
            ("@renderer".to_owned(), "src/renderer/src".to_owned()),
            ("@renderer/*".to_owned(), "src/renderer/src/*".to_owned()),
            ("x".to_owned(), "y".to_owned()),
            ("x/*".to_owned(), "y/*".to_owned()),
        ]
    );
    assert!(vite_aliases(&fixture.path("missing.ts"), fixture.root()).is_empty());
    let outside = Fixture::new(&[("vite.config.js", "export default { alias: { x: '../up' } }")]);
    let aliases = vite_aliases(&outside.path("vite.config.js"), Path::new("Z:/elsewhere"));
    assert!(aliases[0].target.ends_with("up"));
    let config = outside.path("vite.config.js");
    let orphan = vite_aliases(&config, config.parent().unwrap());
    assert_eq!(orphan[0].target, "../up");
}

#[test]
fn env_files() {
    let values = parse_env(
        "# comment\n\nVITE_A=1\nexport VITE_B = \"two words\"\nVITE_C='single'\nVITE_D=plain # trailing\nnot an assignment\nVITE_E=\"\n",
    );
    assert_eq!(values["VITE_A"], "1");
    assert_eq!(values["VITE_B"], "two words");
    assert_eq!(values["VITE_C"], "single");
    assert_eq!(values["VITE_D"], "plain");
    assert_eq!(values["VITE_E"], "\"");
    assert_eq!(values.len(), 5);
    let fixture = Fixture::new(&[
        (".env", "A=base\nB=base"),
        (".env.production", "B=production\nC=production"),
        (".env.production.local", "C=local"),
    ]);
    let env = load_env(fixture.root());
    assert_eq!(env["A"], "base");
    assert_eq!(env["B"], "production");
    assert_eq!(env["C"], "local");
}

#[test]
fn existing_files() {
    let fixture = Fixture::new(&[("b.txt", "")]);
    assert_eq!(
        first_existing(fixture.root(), &["a.txt", "b.txt"]),
        Some(fixture.path("b.txt"))
    );
    assert_eq!(first_existing(fixture.root(), &["a.txt"]), None);
    assert!(read_jsonc(&fixture.path("missing.json")).is_none());
}

#[test]
fn tsconfig_decorator_settings() {
    let fixture = Fixture::new(&[
        (
            "tsconfig.json",
            "{ \"compilerOptions\": { \"strict\": true } }",
        ),
        (
            "tsconfig.node.json",
            "{ \"compilerOptions\": { \"experimentalDecorators\": true, \"emitDecoratorMetadata\": true } }",
        ),
    ]);
    let options = |root: &Path| compiler_options(root, &root.join("main.ts"));
    assert_eq!(tsconfig_decorators(&options(fixture.root())), (true, true));
    let plain = Fixture::new(&[(
        "tsconfig.json",
        "{ \"compilerOptions\": { \"experimentalDecorators\": true } }",
    )]);
    assert_eq!(tsconfig_decorators(&options(plain.root())), (true, false));
    assert_eq!(
        tsconfig_decorators(&options(Fixture::new(&[]).root())),
        (false, false)
    );
}

/// Settings come through `extends`, and from the configuration above the
/// main source when none is at the root, as VS Code keeps
/// `src/tsconfig.json` extending `src/tsconfig.base.json`.
#[test]
fn compiler_options_follow_extends_and_the_source_configuration() {
    let fixture = Fixture::new(&[
        (
            "src/tsconfig.base.json",
            "{ \"compilerOptions\": { \"experimentalDecorators\": true, \"useDefineForClassFields\": false, \"target\": \"ES2024\", \"jsx\": \"react\" } }",
        ),
        (
            "src/tsconfig.json",
            "{ \"extends\": \"./tsconfig.base.json\", \"compilerOptions\": { \"jsx\": \"preserve\" } }",
        ),
        ("src/main.ts", ""),
        (
            "other/tsconfig.json",
            "{ \"compilerOptions\": { \"experimentalDecorators\": false } }",
        ),
    ]);
    let options = compiler_options(fixture.root(), &fixture.path("src/main.ts"));
    assert_eq!(options.len(), 1);
    assert_eq!(tsconfig_decorators(&options), (true, false));
    let typescript = tsconfig_modules(&options);
    assert!(typescript.assign_class_fields);
    // The configuration's own settings come after its base's.
    assert_eq!(typescript.jsx.as_deref(), Some("preserve"));
    // Configurations at the root come first.
    let rooted = Fixture::new(&[
        ("tsconfig.json", "{ \"extends\": [\"./a.json\", \"./b\"] }"),
        (
            "a.json",
            "{ \"compilerOptions\": { \"target\": \"es5\", \"verbatimModuleSyntax\": true } }",
        ),
        (
            "b.json",
            "{ \"compilerOptions\": { \"target\": \"ESNext\" } }",
        ),
        (
            "src/tsconfig.json",
            "{ \"compilerOptions\": { \"useDefineForClassFields\": false } }",
        ),
    ]);
    let options = compiler_options(rooted.root(), &rooted.path("src/main.ts"));
    let typescript = tsconfig_modules(&options);
    assert!(typescript.verbatim_module_syntax);
    assert!(
        !typescript.assign_class_fields,
        "ESNext from the later base"
    );
}

/// `useDefineForClassFields` as given, or as `tsc` defaults it from the
/// target: defined from ES2022, assigned before, and defined without a
/// configuration.
#[test]
fn class_fields_follow_use_define_or_the_target() {
    let assigns = |json: &str| tsconfig_assigns_fields(&[serde_json::from_str(json).unwrap()]);
    assert!(assigns(
        r#"{"useDefineForClassFields": false, "target": "ESNext"}"#
    ));
    assert!(!assigns(
        r#"{"useDefineForClassFields": true, "target": "es5"}"#
    ));
    assert!(!assigns(r#"{"target": "ES2022"}"#));
    assert!(!assigns(r#"{"target": "es2024"}"#));
    assert!(!assigns(r#"{"target": "esnext"}"#));
    assert!(assigns(r#"{"target": "ES2021"}"#));
    assert!(assigns(r#"{"target": "es6"}"#));
    assert!(assigns(r#"{}"#));
    assert!(!tsconfig_assigns_fields(&[]));
}
