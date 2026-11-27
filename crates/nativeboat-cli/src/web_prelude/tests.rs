use super::*;

#[test]
fn the_prelude_runs_once_over_the_runtime_arguments() {
    let root = Path::new("C:/app");
    let prelude = source(&path(root)).unwrap();
    assert_eq!(prelude.kind, ModuleKind::CommonJs);
    assert!(
        prelude
            .text
            .starts_with("'use strict';\nconst prelude = globalThis.__nativeboat_web;")
    );
    assert!(prelude.text.contains("delete globalThis.__nativeboat_web;"));
    assert!(prelude.text.contains("const web = {"));
    assert!(prelude.text.contains(
        "})(prelude[0], prelude[1], prelude[2]);\n} else if (globalThis.__nativeboat_worker) {\n"
    ));
    // A worker's realm runs the worker's global scope instead.
    assert!(
        prelude
            .text
            .contains("delete globalThis.__nativeboat_worker;")
    );
    assert!(prelude.text.contains("class DedicatedWorkerGlobalScope"));
    assert!(prelude.text.ends_with(
        "})(worker[0], worker[1], worker[2], worker[3], worker[4], worker[5], worker[6]);\n}\n"
    ));
    assert!(source(Path::new("C:/app/src/web-prelude.cjs")).is_none());
}
