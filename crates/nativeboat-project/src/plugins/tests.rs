use super::*;
use crate::tests::fixture::Fixture;

const TSCONFIG: &str = r#"{
  // The extension's own build emits src/ into out/.
  "extends": "../tsconfig.base.json",
  "compilerOptions": { "rootDir": "./src", "outDir": "./out" },
  "include": ["src/**/*"]
}"#;

fn sources(entries: &[Entry], root: &Path) -> Vec<(String, Vec<String>)> {
    entries
        .iter()
        .map(|entry| {
            (
                crate::relative(root, &entry.source),
                entry.runtime_paths.clone(),
            )
        })
        .collect()
}

#[test]
fn plugin_mains_compile_from_the_sources_their_tsconfig_emits() {
    let fixture = Fixture::new(&[
        (
            "plugins/tsconfig.base.json",
            r#"{"compilerOptions": {"strict": true}}"#,
        ),
        // Emitted by its own build: `./out/main` is src/main.ts.
        (
            "plugins/git/package.json",
            r#"{"name": "git", "main": "./out/main"}"#,
        ),
        ("plugins/git/tsconfig.json", TSCONFIG),
        ("plugins/git/src/main.ts", "export function activate() {}"),
        // A client and a server package, each with its own configuration;
        // the server's root comes from its include pattern.
        (
            "plugins/json/package.json",
            r#"{"name": "json", "main": "./client/out/node/client"}"#,
        ),
        (
            "plugins/json/client/tsconfig.json",
            r#"{"compilerOptions": {"rootDir": "./src", "outDir": "./out"}}"#,
        ),
        ("plugins/json/client/src/node/client.ts", "export {}"),
        (
            "plugins/json/server/package.json",
            r#"{"name": "server", "main": "./out/node/server"}"#,
        ),
        (
            "plugins/json/server/tsconfig.json",
            r#"{"compilerOptions": {"outDir": "./out"}, "include": ["src/**/*.ts"]}"#,
        ),
        ("plugins/json/server/src/node/server.ts", "export {}"),
        // Shipped JavaScript is its own source; a folder main is its index.
        ("plugins/plain/package.json", r#"{"main": "lib"}"#),
        ("plugins/plain/lib/index.js", "module.exports = 1;"),
        // No main: declarative packages have no code.
        ("plugins/theme/package.json", r#"{"name": "theme"}"#),
        // Left out by an exclusion, and dependencies are never plugins.
        ("plugins/tests-a/package.json", r#"{"main": "./out/a"}"#),
        (
            "plugins/git/node_modules/dep/package.json",
            r#"{"main": "index.js"}"#,
        ),
        ("plugins/git/node_modules/dep/index.js", ""),
        // A main with nothing to compile from.
        ("plugins/broken/package.json", r#"{"main": "./out/gone"}"#),
    ]);
    let root = fixture.root();
    let (entries, warnings) = entries(
        root,
        &["plugins/*".to_owned(), "!plugins/tests-*".to_owned()],
    );
    assert_eq!(
        sources(&entries, root),
        [
            (
                "plugins/git/src/main.ts".to_owned(),
                vec![
                    "plugins/git/out/main.js".to_owned(),
                    "plugins/git/out/main".to_owned()
                ]
            ),
            (
                "plugins/json/client/src/node/client.ts".to_owned(),
                vec![
                    "plugins/json/client/out/node/client.js".to_owned(),
                    "plugins/json/client/out/node/client".to_owned()
                ]
            ),
            (
                "plugins/json/server/src/node/server.ts".to_owned(),
                vec![
                    "plugins/json/server/out/node/server.js".to_owned(),
                    "plugins/json/server/out/node/server".to_owned()
                ]
            ),
            (
                "plugins/plain/lib/index.js".to_owned(),
                vec![
                    "plugins/plain/lib/index.js".to_owned(),
                    "plugins/plain/lib/index".to_owned()
                ]
            ),
        ]
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("plugins/broken"), "{warnings:?}");
    // Without patterns nothing is a plugin.
    assert!(super::entries(root, &[]).0.is_empty());
}

#[test]
fn plugin_sources_compile_to_commonjs_as_their_tsconfig_module_says() {
    let fixture = Fixture::new(&[
        (
            "base.json",
            r#"{"compilerOptions": {"module": "commonjs"}}"#,
        ),
        ("a/tsconfig.json", r#"{"extends": "../base.json"}"#),
        ("a/src/main.ts", ""),
        ("b/package.json", r#"{"type": "module"}"#),
        (
            "b/tsconfig.json",
            r#"{"compilerOptions": {"module": "nodenext"}}"#,
        ),
        ("b/src/main.ts", ""),
        ("c/package.json", r#"{"name": "c"}"#),
        (
            "c/tsconfig.json",
            r#"{"compilerOptions": {"module": "NodeNext"}}"#,
        ),
        ("c/src/main.ts", ""),
        (
            "d/tsconfig.json",
            r#"{"compilerOptions": {"module": "es2020"}}"#,
        ),
        ("d/src/main.ts", ""),
        ("e/src/main.ts", ""),
    ]);
    let root = fixture.root();
    let emits = |name: &str| emits_commonjs(root, &root.join(name).join("src/main.ts"));
    assert!(emits("a"));
    assert!(!emits("b"));
    assert!(emits("c"));
    assert!(!emits("d"));
    assert!(!emits("e"));
}

#[test]
fn plugin_settings_are_lists_of_globs() {
    assert_eq!(
        patterns(&serde_json::json!(["a/*", "!a/b"])),
        Some(vec!["a/*".to_owned(), "!a/b".to_owned()])
    );
    assert_eq!(patterns(&serde_json::json!("a/*")), None);
    assert_eq!(patterns(&serde_json::json!(["a", 1])), None);
}
