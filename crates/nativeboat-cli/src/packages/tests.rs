use super::*;

#[test]
fn replaces_ajv_entry_files_beside_the_originals() {
    let entry = Path::new("C:/app/node_modules/.pnpm/ajv@8.20.0/node_modules/ajv/dist/2020.js");
    let virtual_path = replacement(entry).unwrap();
    assert_eq!(
        virtual_path,
        PathBuf::from(format!("{}{SUFFIX}", entry.display()))
    );
    let entry_source = source(&virtual_path).unwrap();
    assert_eq!(entry_source.kind, ModuleKind::CommonJs);
    assert!(
        entry_source
            .text
            .contains("module.exports.Ajv2020 = Ajv2020")
    );

    let windows = Path::new(r"C:\app\node_modules\ajv-formats\dist\index.js");
    let formats = source(&replacement(windows).unwrap()).unwrap();
    assert!(formats.text.contains("require('./formats')"));
}

#[test]
fn replaces_fflates_node_entry_with_main_thread_operations() {
    let entry =
        Path::new("C:/app/node_modules/.pnpm/fflate@0.8.2/node_modules/fflate/lib/node.cjs");
    let replaced = source(&replacement(entry).unwrap()).unwrap();
    assert_eq!(replaced.kind, ModuleKind::CommonJs);
    // The synchronous API is the package's own, beside the replacement.
    assert!(replaced.text.contains("require('./index.cjs')"));
    assert!(!replaced.text.contains("worker_threads"));
    assert_eq!(
        replacement(Path::new("C:/app/node_modules/fflate/lib/index.cjs")),
        None
    );
}

#[test]
fn leaves_other_package_files_unchanged() {
    assert_eq!(
        replacement(Path::new("C:/app/node_modules/ajv/lib/ajv.js")),
        None
    );
    assert_eq!(
        replacement(Path::new("C:/app/node_modules/not-ajv/dist/2020.js")),
        None
    );
    assert!(source(Path::new("C:/app/node_modules/ajv/dist/2020.js")).is_none());
    assert!(source(Path::new("C:/app/src/2020.js.nativeboat-package.cjs")).is_none());
}

#[test]
fn resolves_the_shared_interpreter_from_replacements_only() {
    let entry = replacement(Path::new("C:/app/node_modules/ajv/dist/ajv.js")).unwrap();
    let interpreter = request(&entry, JSON_SCHEMA).unwrap();
    assert!(
        interpreter
            .to_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("/ajv/dist/nativeboat-json-schema.js.nativeboat-package.cjs")
    );
    assert!(source(&interpreter).unwrap().text.contains("class Ajv2020"));
    assert_eq!(request(&entry, "./core"), None);
    assert_eq!(request(Path::new("C:/app/src/index.js"), JSON_SCHEMA), None);
    assert_eq!(
        request(
            Path::new("C:/app/src/index.js.nativeboat-package.cjs"),
            JSON_SCHEMA
        ),
        None
    );
}
