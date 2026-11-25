use super::*;
use crate::tests::Fixture;
use crate::{BundleOptions, Entry, Target, bundle};
use shimforge::{Session, mock};

fn options(fixture: &Fixture) -> BundleOptions {
    BundleOptions {
        root: fixture.root().to_owned(),
        target: Target::Main,
        aliases: vec![],
        transpile: Default::default(),
        emits: vec![],
        mappings: Vec::new(),
    }
}

#[test]
fn computed_resolution_packages_installed_public_platform_files_without_loading_them() {
    let fixture = Fixture::new(&[
        ("package.json", r#"{"dependencies":{"loader":"1"}}"#),
        (
            "main.js",
            "import {tool} from 'loader'; globalThis.tool=tool;",
        ),
        (
            "node_modules/loader/package.json",
            r#"{"main":"lib/index.js","optionalDependencies":{"@tools/windows":"1","missing":"1","other-os":"1","other-cpu":"1"},"dependencies":{"@tools/windows":"1","builtin":"1"}}"#,
        ),
        (
            "node_modules/loader/lib/index.js",
            "import {createRequire as make} from 'node:module'; const require=make(import.meta.url); const name='@tools/windows'; export const tool=require.resolve(`${name}/bin/tool.exe`);",
        ),
        (
            "node_modules/@tools/windows/package.json",
            r#"{"os":["win32"],"cpu":["x64"],"exports":{"./bin/*":"./bin/*"}}"#,
        ),
        (
            "node_modules/@tools/windows/bin/tool.exe",
            "MZ generic test binary",
        ),
        (
            "node_modules/@tools/windows/private.js",
            "this is not valid JavaScript and must never be parsed",
        ),
        (
            "node_modules/@tools/windows/node_modules/ignored.txt",
            "nested dependency excluded",
        ),
        (
            "node_modules/@tools/windows/.git/ignored.txt",
            "metadata excluded",
        ),
        ("node_modules/other-os/package.json", r#"{"os":["!win32"]}"#),
        ("node_modules/other-os/bin/tool", "other OS"),
        (
            "node_modules/other-cpu/package.json",
            r#"{"cpu":["arm64"]}"#,
        ),
        ("node_modules/other-cpu/bin/tool", "other architecture"),
    ]);
    let result = bundle(
        &[Entry {
            path: fixture.path("main.js"),
            source: None,
        }],
        &options(&fixture),
    )
    .unwrap();
    assert_eq!(
        result.modules.len(),
        2,
        "resolve-only dependencies are never evaluated or compiled"
    );
    let dependencies = &result.modules[1].dependencies;
    assert!(dependencies.contains(&(
        "@tools/windows/bin/tool.exe".into(),
        Resolution::File("node_modules/@tools/windows/bin/tool.exe".into())
    )));
    assert!(
        !dependencies
            .iter()
            .any(|(specifier, _)| specifier.contains("private")
                || specifier.contains("other-os")
                || specifier.contains("other-cpu"))
    );
    assert_eq!(result.assets.len(), 1);
    assert_eq!(
        result.assets[0].url,
        "/node_modules/@tools/windows/bin/tool.exe"
    );
    assert!(result.warnings[0].contains("dynamic require.resolve"));
}

#[test]
fn platform_declarations_preserve_positive_and_negative_rules() {
    for (value, expected) in [
        (None, true),
        (Some(serde_json::json!("win32")), true),
        (Some(serde_json::json!("darwin")), false),
        (Some(serde_json::json!(["any"])), true),
        (Some(serde_json::json!(["!darwin"])), true),
        (Some(serde_json::json!(["win32", "!win32"])), false),
        (Some(serde_json::json!(["!any"])), false),
        (Some(serde_json::json!(false)), false),
        (Some(serde_json::json!([])), true),
    ] {
        assert_eq!(compatible(value.as_ref(), "win32"), expected, "{value:?}");
    }
}

#[test]
fn resource_walk_skips_cycles_and_reports_unreadable_directories() {
    let fixture = Fixture::new(&[("tree/file.bin", "data")]);
    fixture.junction("tree/again", "tree");
    assert_eq!(
        files(&fixture.path("tree")).unwrap(),
        vec![fixture.path("tree/file.bin")]
    );
    assert!(files(&fixture.path("missing")).is_err());
    let mut session = Session::new();
    let listing = mock!(
        session,
        super::entries,
        fn(&Path) -> std::io::Result<Vec<PathBuf>>
    );
    listing
        .expect()
        .returning(|directory| Ok(vec![directory.join("nonexistent")]));
    assert!(files(&fixture.path("tree")).unwrap().is_empty());
}

#[test]
fn absent_manifests_existing_static_edges_and_enumeration_failures_are_preserved() {
    let fixture = Fixture::new(&[
        ("main.js", "require.resolve(name);"),
        ("tool/index.js", "module.exports=1;"),
    ]);
    let entry = Entry {
        path: fixture.path("main.js"),
        source: None,
    };
    assert!(
        bundle(std::slice::from_ref(&entry), &options(&fixture))
            .unwrap()
            .assets
            .is_empty()
    );
    fixture.write("package.json", br#"{"dependencies":{"tool":"1","fs":"1"}}"#);
    fixture.junction("node_modules/tool", "tool");
    fixture.write(
        "main.js",
        b"require.resolve('tool'); require.resolve(name);",
    );
    let result = bundle(std::slice::from_ref(&entry), &options(&fixture)).unwrap();
    assert!(matches!(
        result.modules[0].dependencies[0].1,
        Resolution::Module(_)
    ));
    assert!(
        result.modules[0]
            .dependencies
            .iter()
            .any(|(name, _)| name == "tool/index.js")
    );
    let mut session = Session::new();
    let enumerating = mock!(
        session,
        super::files,
        fn(&Path) -> std::io::Result<Vec<PathBuf>>
    );
    enumerating
        .expect()
        .returning(|_| Err(std::io::Error::other("inaccessible")));
    let result = bundle(&[entry], &options(&fixture)).unwrap();
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("cannot enumerate dependency tool: inaccessible"))
    );
}
