use super::*;
use crate::tests::fixture::Fixture;

/// Relative paths name files next to the module; other paths name files in
/// the folders above it, up to its package's folder, and JavaScript names
/// stand for their TypeScript sources.
#[test]
fn worker_scripts_are_found_next_to_and_above_the_module_naming_them() {
    let fixture = Fixture::new(&[
        ("app/package.json", "{}"),
        ("app/src/vs/editor/browser/service.ts", ""),
        ("app/src/vs/editor/browser/near.js", ""),
        ("app/src/vs/editor/common/workerMain.ts", ""),
        ("app/src/plain.mjs", ""),
        ("app/src/typed.mts", ""),
        ("app/outside.js", ""),
        ("elsewhere.js", ""),
    ]);
    let from = fixture.path("app/src/vs/editor/browser/service.ts");
    let found = |name: &str| locate(&from, name);
    let real = |path: &str| nativeboat_bundler::resolve::realpath(&fixture.path(path));
    assert_eq!(
        found("./near.js"),
        Some(real("app/src/vs/editor/browser/near.js"))
    );
    assert_eq!(
        found("vs/editor/common/workerMain.js"),
        Some(real("app/src/vs/editor/common/workerMain.ts"))
    );
    assert_eq!(found("plain.mjs"), Some(real("app/src/plain.mjs")));
    assert_eq!(found("typed.mjs"), Some(real("app/src/typed.mts")));
    assert_eq!(found("outside.js"), Some(real("app/outside.js")));
    // The package's folder bounds the search, and relative paths are not
    // searched for above the module.
    assert_eq!(found("elsewhere.js"), None);
    assert_eq!(found("./plain.mjs"), None);
    assert_eq!(found("vs/editor/common/missing.js"), None);
}
