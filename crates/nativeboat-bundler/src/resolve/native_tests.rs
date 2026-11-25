use super::*;
use crate::tests::Fixture;

#[test]
fn native_imports_exclude_require_and_preserve_process_specific_fields() {
    let fixture = Fixture::new(&[
        (
            "node_modules/conditional/package.json",
            r#"{"exports":{"require":"./wrong.js","import":"./esm.js"}}"#,
        ),
        ("node_modules/conditional/wrong.js", "module.exports = 1;"),
        ("node_modules/conditional/esm.js", "export const value = 2;"),
        (
            "node_modules/import-first/package.json",
            r#"{"exports":{"import":"./esm.js","require":"./cjs.js"}}"#,
        ),
        (
            "node_modules/import-first/esm.js",
            "export const value = 2;",
        ),
        ("node_modules/import-first/cjs.js", "module.exports = 1;"),
        (
            "node_modules/fields/package.json",
            r#"{"main":"./main.js","module":"./module.js","browser":"./browser.js"}"#,
        ),
        ("node_modules/fields/main.js", "export const value = 1;"),
        ("node_modules/fields/module.js", "export const value = 2;"),
        ("node_modules/fields/browser.js", "export const value = 3;"),
        (
            "node_modules/mapped/package.json",
            r#"{"browser":{"./file":"./native.js"}}"#,
        ),
        ("node_modules/mapped/file.js", "export const value = 1;"),
        ("node_modules/mapped/native.js", "export const value = 2;"),
    ]);
    let from = fixture.path("main.js");
    for target in [
        Target::MainImport,
        Target::RendererImport,
        Target::MainRequire,
        Target::RendererRequire,
    ] {
        let resolver = Resolver {
            root: fixture.root().to_path_buf(),
            target,
            aliases: vec![],
        };
        let selected = if matches!(target, Target::MainRequire | Target::RendererRequire) {
            "wrong"
        } else {
            "esm"
        };
        assert_eq!(
            resolver.resolve("conditional", &from).unwrap(),
            Resolved::File(fixture.path(&format!("node_modules/conditional/{selected}.js")))
        );
        let selected = if matches!(target, Target::MainRequire | Target::RendererRequire) {
            "cjs"
        } else {
            "esm"
        };
        assert_eq!(
            resolver.resolve("import-first", &from).unwrap(),
            Resolved::File(fixture.path(&format!("node_modules/import-first/{selected}.js")))
        );
        let field = if target.is_renderer() {
            "browser"
        } else {
            "main"
        };
        assert_eq!(
            resolver.resolve("fields", &from).unwrap(),
            Resolved::File(fixture.path(&format!("node_modules/fields/{field}.js")))
        );
        let mapped = if target.is_renderer() {
            "native"
        } else {
            "file"
        };
        assert_eq!(
            resolver.resolve("mapped/file", &from).unwrap(),
            Resolved::File(fixture.path(&format!("node_modules/mapped/{mapped}.js")))
        );
    }
}
