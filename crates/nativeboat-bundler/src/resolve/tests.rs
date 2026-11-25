use super::*;
use crate::tests::Fixture;

fn resolver(fixture: &Fixture, target: Target) -> Resolver {
    Resolver {
        root: fixture.root().to_path_buf(),
        target,
        aliases: vec![
            Alias {
                pattern: "@/*".to_owned(),
                target: "src/*".to_owned(),
                kind: AliasKind::Path,
            },
            Alias {
                pattern: "~utils".to_owned(),
                target: "src/utils".to_owned(),
                kind: AliasKind::Path,
            },
            Alias {
                pattern: "*.icon".to_owned(),
                target: "icons/*.svg".to_owned(),
                kind: AliasKind::Path,
            },
        ],
    }
}

fn file(fixture: &Fixture, relative: &str) -> Resolved {
    Resolved::File(normalize(&fixture.path(relative)))
}

#[test]
fn builtins() {
    assert_eq!(builtin("ced"), Some("ced".into()));
    assert_eq!(builtin("node:ced"), None);
    assert_eq!(builtin("ced/index.js"), None);
    assert_eq!(builtin("keytar"), Some("keytar".into()));
    assert_eq!(builtin("node:keytar"), None);
    assert_eq!(builtin("keytar/lib/keytar.js"), None);
    assert_eq!(builtin("native-keymap"), Some("native-keymap".into()));
    assert_eq!(builtin("node:native-keymap"), None);
    assert_eq!(builtin("native-keymap/index.js"), None);
    assert_eq!(builtin("fs"), Some("fs".to_owned()));
    assert_eq!(builtin("node:fs/promises"), Some("fs/promises".to_owned()));
    assert_eq!(builtin("original-fs"), Some("fs".to_owned()));
    assert_eq!(builtin("node:original-fs"), None);
    assert_eq!(builtin("electron"), Some("electron".to_owned()));
    assert_eq!(
        builtin("electron/renderer"),
        Some("electron/renderer".to_owned())
    );
    assert_eq!(builtin("react"), None);
    let fixture = Fixture::new(&[("a.js", "")]);
    let resolved = resolver(&fixture, Target::Main).resolve("node:path", &fixture.path("a.js"));
    assert_eq!(resolved, Ok(Resolved::Builtin("path".to_owned())));
}

#[test]
fn node_resolution_names_preserve_prefixes_aliases_and_user_packages() {
    let fixture = Fixture::new(&[
        ("entry.js", ""),
        ("node_modules/test/package.json", r#"{"main":"index.js"}"#),
        ("node_modules/test/index.js", ""),
    ]);
    let mut resolver = resolver(&fixture, Target::MainRequire);
    for name in [
        "path",
        "node:path",
        "_http_common",
        "node:_tls_wrap",
        "inspector/promises",
        "stream/consumers",
        "node:sea",
        "node:sqlite",
        "node:test",
        "node:test/reporters",
    ] {
        assert_eq!(resolver.node_builtin_specifier(name), Some(name.into()));
    }
    for name in [
        "sea",
        "sqlite",
        "test",
        "test/reporters",
        "node:unknown",
        "electron",
        "node:electron",
        "keytar",
        "node:keytar",
        "ordinary",
    ] {
        assert_eq!(resolver.node_builtin_specifier(name), None, "{name}");
    }
    assert_eq!(
        resolver.resolve("test", &fixture.path("entry.js")).unwrap(),
        file(&fixture, "node_modules/test/index.js")
    );
    assert_eq!(
        resolver
            .resolve("node:test", &fixture.path("entry.js"))
            .unwrap(),
        Resolved::Builtin("test".into())
    );
    resolver.aliases.extend([
        Alias {
            pattern: "prefixed".into(),
            target: "node:path".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "core/*".into(),
            target: "node:*".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "path".into(),
            target: "ordinary".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "fs".into(),
            target: "node:path".into(),
            kind: AliasKind::Path,
        },
    ]);
    assert_eq!(
        resolver.node_builtin_specifier("prefixed"),
        Some("node:path".into())
    );
    assert_eq!(
        resolver.node_builtin_specifier("core/fs"),
        Some("node:fs".into())
    );
    assert_eq!(resolver.node_builtin_specifier("path"), None);
    assert_eq!(resolver.node_builtin_specifier("fs"), None);
}

#[test]
fn linked_packages_resolve_dependencies_from_their_real_location() {
    let fixture = Fixture::new(&[
        ("entry.js", ""),
        (
            "node_modules/.pnpm/consumer/node_modules/consumer/package.json",
            r#"{"main":"index.js"}"#,
        ),
        (
            "node_modules/.pnpm/consumer/node_modules/consumer/index.js",
            "require('validator/new.js')",
        ),
        (
            "node_modules/.pnpm/validator-v2/node_modules/validator/new.js",
            "",
        ),
        ("node_modules/validator/old.js", ""),
        ("workspace/src/index.ts", ""),
        ("peer store @1(pkg@2)/index.js", ""),
    ]);
    fixture.junction(
        "node_modules/consumer",
        "node_modules/.pnpm/consumer/node_modules/consumer",
    );
    fixture.junction(
        "node_modules/.pnpm/consumer/node_modules/validator",
        "node_modules/.pnpm/validator-v2/node_modules/validator",
    );
    fixture.junction("node_modules/local-workspace", "workspace");
    fixture.junction("node_modules/peer-linked", "peer store @1(pkg@2)");
    let resolver = resolver(&fixture, Target::Main);
    let physical = fixture.path("node_modules/.pnpm/consumer/node_modules/consumer/index.js");
    assert_eq!(
        resolver
            .resolve("consumer", &fixture.path("entry.js"))
            .unwrap(),
        Resolved::File(physical)
    );
    let logical = fixture.path("node_modules/consumer/index.js");
    assert_eq!(
        resolver.resolve("validator/new.js", &logical).unwrap(),
        file(
            &fixture,
            "node_modules/.pnpm/validator-v2/node_modules/validator/new.js"
        )
    );
    assert_eq!(
        resolver
            .resolve("local-workspace/src/index.js", &fixture.path("entry.js"))
            .unwrap(),
        file(&fixture, "workspace/src/index.ts")
    );
    assert!(
        resolver
            .resolve("validator/new.js", &fixture.path("entry.js"))
            .is_err()
    );
    assert_eq!(
        resolver
            .resolve("peer-linked", &fixture.path("entry.js"))
            .unwrap(),
        file(&fixture, "peer store @1(pkg@2)/index.js")
    );
}

#[test]
#[ignore = "requires installed unchanged MarkText dependencies; set NATIVEBOAT_MARKTEXT_PROJECT"]
fn unchanged_marktext_pnpm_dependencies_use_their_real_package_versions() {
    let root = PathBuf::from(std::env::var_os("NATIVEBOAT_MARKTEXT_PROJECT").unwrap());
    let resolver = Resolver {
        root: root.join("packages/desktop"),
        target: Target::Main,
        aliases: Vec::new(),
    };
    let Resolved::File(conf) = resolver
        .resolve("conf", &root.join("packages/desktop/src/main/index.ts"))
        .unwrap()
    else {
        panic!("conf must resolve to a file")
    };
    let Resolved::File(validator) = resolver.resolve("ajv/dist/2020.js", &conf).unwrap() else {
        panic!("Ajv must resolve to a file")
    };
    let Resolved::File(hoisted) = resolver
        .resolve("ajv/package.json", &root.join("package.json"))
        .unwrap()
    else {
        panic!("hoisted Ajv must resolve to a file")
    };
    let nested = read_json(
        &validator
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("package.json"),
    )
    .unwrap();
    let hoisted = read_json(&hoisted).unwrap();
    assert!(nested["version"].as_str().unwrap().starts_with("8."));
    assert!(hoisted["version"].as_str().unwrap().starts_with("6."));
    assert_eq!(realpath(&conf), conf);
    assert_eq!(realpath(&validator), validator);
    println!("MarkText: conf resolves its linked Ajv 8 independently of hoisted Ajv 6");
}

#[test]
fn canonical_paths_preserve_windows_roots_and_fallback_for_synthetic_entries() {
    for (path, expected) in [
        (r"\\?\C:\folder\file.js", r"C:\folder\file.js"),
        (r"\\?\UNC\server\share\file.js", r"\\server\share\file.js"),
        (r"C:\folder\file.js", r"C:\folder\file.js"),
        ("folder/file.js", "folder/file.js"),
    ] {
        assert_eq!(plain_path(Path::new(path)), PathBuf::from(expected));
    }
    assert_eq!(
        realpath(Path::new("missing/../synthetic.js")),
        PathBuf::from("synthetic.js")
    );
    let fixture = Fixture::new(&[("entry.js", "")]);
    let mut session = shimforge::Session::new();
    let canonical = shimforge::mock!(
        session,
        std::fs::canonicalize::<&Path>,
        fn(&Path) -> std::io::Result<PathBuf>
    );
    canonical
        .expect()
        .returning(|_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
    assert_eq!(
        realpath(&fixture.path("entry.js")),
        fixture.path("entry.js")
    );
}

#[test]
fn relative_paths_extensions_and_indexes() {
    let fixture = Fixture::new(&[
        ("src/a.ts", ""),
        ("src/a.js", ""),
        ("src/b.json", "{}"),
        ("src/dir/index.tsx", ""),
        ("src/main/package.json", "{\"main\": \"lib/entry\"}"),
        ("src/main/lib/entry.js", ""),
        ("src/broken/package.json", "{\"main\": \"missing\"}"),
        ("src/broken/index.js", ""),
        ("src/garbage/package.json", "not json"),
        ("src/garbage/index.js", ""),
        ("src/exact.css", ""),
        ("src/index.js", ""),
        ("index.js", ""),
    ]);
    let resolver = resolver(&fixture, Target::Main);
    let from = fixture.path("src/x.ts");
    let resolve = |specifier: &str| resolver.resolve(specifier, &from).unwrap();
    assert_eq!(resolve("./a"), file(&fixture, "src/a.ts"));
    assert_eq!(resolve("./a.js"), file(&fixture, "src/a.js"));
    assert_eq!(resolve("./b"), file(&fixture, "src/b.json"));
    assert_eq!(resolve("./dir"), file(&fixture, "src/dir/index.tsx"));
    assert_eq!(resolve("./main"), file(&fixture, "src/main/lib/entry.js"));
    assert_eq!(resolve("./broken"), file(&fixture, "src/broken/index.js"));
    assert_eq!(resolve("./garbage"), file(&fixture, "src/garbage/index.js"));
    assert_eq!(resolve("./exact.css"), file(&fixture, "src/exact.css"));
    assert_eq!(resolve("../index"), file(&fixture, "index.js"));
    assert_eq!(resolve("./dir/./../a"), file(&fixture, "src/a.ts"));
    assert_eq!(
        resolver
            .resolve(".", &fixture.path("src/dir/x.js"))
            .unwrap(),
        file(&fixture, "src/dir/index.tsx")
    );
    assert_eq!(
        resolver
            .resolve("..", &fixture.path("src/dir/x.js"))
            .unwrap(),
        file(&fixture, "src/index.js")
    );
    assert_eq!(resolve("/src/a.js"), file(&fixture, "src/a.js"));
    let absolute = fixture.path("src/a.js").to_string_lossy().into_owned();
    assert_eq!(resolve(&absolute), file(&fixture, "src/a.js"));
    let error = resolver.resolve("./missing", &from).unwrap_err();
    assert!(error.starts_with("cannot resolve \"./missing\" from "));
    assert!(resolver.resolve("./missing.js", &from).is_err());
    let orphan = Resolver {
        root: fixture.root().to_path_buf(),
        target: Target::Main,
        aliases: Vec::new(),
    };
    assert_eq!(
        orphan.resolve("./index", Path::new("")).unwrap(),
        Resolved::File(normalize(&fixture.path("index.js")))
    );
}

#[test]
fn compiled_names_lead_to_typescript_sources() {
    let fixture = Fixture::new(&[
        ("src/ipc.ts", ""),
        ("src/view.tsx", ""),
        ("src/legacy.jsx", ""),
        ("src/worker.mts", ""),
        ("src/preload.cts", ""),
        ("src/both.ts", ""),
        ("src/both.js", ""),
    ]);
    let resolver = resolver(&fixture, Target::Main);
    let from = fixture.path("src/main.ts");
    let resolve = |specifier: &str| resolver.resolve(specifier, &from).unwrap();
    assert_eq!(resolve("./ipc.js"), file(&fixture, "src/ipc.ts"));
    assert_eq!(resolve("./view.js"), file(&fixture, "src/view.tsx"));
    assert_eq!(resolve("./legacy.js"), file(&fixture, "src/legacy.jsx"));
    assert_eq!(resolve("./view.jsx"), file(&fixture, "src/view.tsx"));
    assert_eq!(resolve("./worker.mjs"), file(&fixture, "src/worker.mts"));
    assert_eq!(resolve("./preload.cjs"), file(&fixture, "src/preload.cts"));
    // An existing file wins.
    assert_eq!(resolve("./both.js"), file(&fixture, "src/both.js"));
}

#[test]
fn aliases() {
    let fixture = Fixture::new(&[
        ("src/components/Button.tsx", ""),
        ("src/utils/index.ts", ""),
        ("icons/star.svg", ""),
        ("node_modules/@/package.json", "{}"),
    ]);
    let resolver = resolver(&fixture, Target::Renderer);
    let from = fixture.path("src/App.tsx");
    assert_eq!(
        resolver.resolve("@/components/Button", &from).unwrap(),
        file(&fixture, "src/components/Button.tsx")
    );
    assert_eq!(
        resolver.resolve("~utils", &from).unwrap(),
        file(&fixture, "src/utils/index.ts")
    );
    assert_eq!(
        resolver.resolve("star.icon", &from).unwrap(),
        file(&fixture, "icons/star.svg")
    );
    assert!(resolver.resolve("star.other", &from).is_err());
    assert!(resolver.resolve("~utilsx", &from).is_err());
}

#[test]
fn module_aliases_override_builtins_and_resolve_relative_to_the_importer() {
    let fixture = Fixture::new(&[
        ("node_modules/pathe/package.json", r#"{"main":"index.js"}"#),
        ("node_modules/pathe/index.js", ""),
        ("node_modules/pathe/sub.js", ""),
        ("src/relative.js", ""),
        ("root.js", ""),
    ]);
    let mut resolver = resolver(&fixture, Target::Renderer);
    resolver.aliases.extend([
        Alias {
            pattern: "path".into(),
            target: "pathe".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "path/*".into(),
            target: "pathe/*".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "relative".into(),
            target: "./relative.js".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "fs".into(),
            target: fixture.path("root.js").to_string_lossy().into_owned(),
            kind: AliasKind::Path,
        },
        Alias {
            pattern: "other".into(),
            target: "node:os".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "missing".into(),
            target: "absent".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "missing".into(),
            target: "pathe".into(),
            kind: AliasKind::Specifier,
        },
        Alias {
            pattern: "cycle".into(),
            target: "cycle".into(),
            kind: AliasKind::Specifier,
        },
    ]);
    let from = fixture.path("src/main.ts");
    assert_eq!(
        resolver.resolve("path", &from).unwrap(),
        file(&fixture, "node_modules/pathe/index.js")
    );
    assert_eq!(
        resolver.resolve("path/sub", &from).unwrap(),
        file(&fixture, "node_modules/pathe/sub.js")
    );
    assert_eq!(
        resolver.resolve("node:path", &from).unwrap(),
        Resolved::Builtin("path".into())
    );
    assert_eq!(
        resolver.resolve("relative", &from).unwrap(),
        file(&fixture, "src/relative.js")
    );
    assert_eq!(
        resolver.resolve("fs", &from).unwrap(),
        file(&fixture, "root.js")
    );
    assert_eq!(
        resolver.resolve("other", &from).unwrap(),
        Resolved::Builtin("os".into())
    );
    assert!(
        resolver
            .resolve("missing", &from)
            .unwrap_err()
            .contains("\"missing\"")
    );
    assert!(resolver.resolve("cycle", &from).is_err());
}

#[test]
fn packages_and_main_fields() {
    let fixture = Fixture::new(&[
        (
            "node_modules/plain/package.json",
            "{\"main\": \"lib/index.js\"}",
        ),
        ("node_modules/plain/lib/index.js", ""),
        ("node_modules/plain/sub.js", ""),
        ("node_modules/@scope/pkg/package.json", "{}"),
        ("node_modules/@scope/pkg/index.js", ""),
        ("node_modules/@scope/pkg/deep/file.js", ""),
        ("node_modules/@scope/package.json", "{}"),
        ("node_modules/@scope/index.js", ""),
        (
            "node_modules/fields/package.json",
            "{\"main\": \"main.js\", \"module\": \"module.js\", \"browser\": \"browser.js\"}",
        ),
        ("node_modules/fields/main.js", ""),
        ("node_modules/fields/module.js", ""),
        ("node_modules/fields/browser.js", ""),
        (
            "node_modules/esm/package.json",
            "{\"main\": \"main.js\", \"module\": \"module.js\"}",
        ),
        ("node_modules/esm/main.js", ""),
        ("node_modules/esm/module.js", ""),
        (
            "node_modules/mapped/package.json",
            "{\"browser\": {\"./server.js\": \"./client.js\", \"./relative-entry.js\": \"./client-internal.js\", \"./relative-extensionless\": \"./client-internal.js\", \"./relative-native.js\": false, \"./native\": false, \".\": \"./entry.js\", \"./obj\": 1}}",
        ),
        ("node_modules/mapped/index.js", ""),
        ("node_modules/mapped/entry.js", ""),
        ("node_modules/mapped/server.js", ""),
        ("node_modules/mapped/client.js", ""),
        ("node_modules/mapped/native.js", ""),
        ("node_modules/mapped/obj.js", ""),
        ("node_modules/mapped/relative-entry.js", ""),
        ("node_modules/mapped/relative-extensionless.js", ""),
        ("node_modules/mapped/relative-native.js", ""),
        ("node_modules/mapped/client-internal.js", ""),
        ("node_modules/nomanifest/index.js", ""),
        // As plantuml-encoder maps its main file for browsers.
        (
            "node_modules/mainmapped/package.json",
            "{\"main\": \"index.js\", \"browser\": {\"./index.js\": \"./browser-index.js\", \"./lib/node.js\": \"./lib/browser.js\"}}",
        ),
        ("node_modules/mainmapped/index.js", ""),
        ("node_modules/mainmapped/browser-index.js", ""),
        ("node_modules/mainmapped/lib/node.js", ""),
        ("node_modules/mainmapped/lib/browser.js", ""),
        ("src/deep/nested/file.js", ""),
    ]);
    let main = resolver(&fixture, Target::Main);
    let renderer = resolver(&fixture, Target::Renderer);
    let renderer_require = resolver(&fixture, Target::RendererRequire);
    let from = fixture.path("src/deep/nested/file.js");
    assert_eq!(
        main.resolve("plain", &from).unwrap(),
        file(&fixture, "node_modules/plain/lib/index.js")
    );
    assert_eq!(
        main.resolve("plain/sub", &from).unwrap(),
        file(&fixture, "node_modules/plain/sub.js")
    );
    assert_eq!(
        main.resolve("@scope/pkg", &from).unwrap(),
        file(&fixture, "node_modules/@scope/pkg/index.js")
    );
    assert_eq!(
        main.resolve("@scope/pkg/deep/file", &from).unwrap(),
        file(&fixture, "node_modules/@scope/pkg/deep/file.js")
    );
    assert_eq!(
        main.resolve("@scope", &from).unwrap(),
        file(&fixture, "node_modules/@scope/index.js")
    );
    assert_eq!(
        main.resolve("fields", &from).unwrap(),
        file(&fixture, "node_modules/fields/main.js")
    );
    assert_eq!(
        renderer.resolve("fields", &from).unwrap(),
        file(&fixture, "node_modules/fields/browser.js")
    );
    assert_eq!(
        renderer.resolve("esm", &from).unwrap(),
        file(&fixture, "node_modules/esm/module.js")
    );
    assert_eq!(
        renderer_require.resolve("esm", &from).unwrap(),
        file(&fixture, "node_modules/esm/main.js")
    );
    assert_eq!(
        renderer.resolve("mapped", &from).unwrap(),
        file(&fixture, "node_modules/mapped/entry.js")
    );
    assert_eq!(
        main.resolve("mapped", &from).unwrap(),
        file(&fixture, "node_modules/mapped/index.js")
    );
    assert_eq!(
        renderer.resolve("mapped/server", &from).unwrap(),
        file(&fixture, "node_modules/mapped/client.js")
    );
    // The map replaces the file the package's main field names, and files
    // named by subpath.
    assert_eq!(
        renderer.resolve("mainmapped", &from).unwrap(),
        file(&fixture, "node_modules/mainmapped/browser-index.js")
    );
    assert_eq!(
        main.resolve("mainmapped", &from).unwrap(),
        file(&fixture, "node_modules/mainmapped/index.js")
    );
    assert_eq!(
        renderer.resolve("mainmapped/lib/node", &from).unwrap(),
        file(&fixture, "node_modules/mainmapped/lib/browser.js")
    );
    assert_eq!(
        renderer
            .resolve(
                "./relative-entry.js",
                &fixture.path("node_modules/mapped/index.js")
            )
            .unwrap(),
        file(&fixture, "node_modules/mapped/client-internal.js")
    );
    assert_eq!(
        renderer
            .resolve(
                "./relative-extensionless.js",
                &fixture.path("node_modules/mapped/index.js")
            )
            .unwrap(),
        file(&fixture, "node_modules/mapped/client-internal.js")
    );
    assert_eq!(
        renderer
            .resolve(
                "./relative-native.js",
                &fixture.path("node_modules/mapped/index.js")
            )
            .unwrap(),
        Resolved::Empty
    );
    assert_eq!(
        main.resolve(
            "./relative-entry.js",
            &fixture.path("node_modules/mapped/index.js")
        )
        .unwrap(),
        file(&fixture, "node_modules/mapped/relative-entry.js")
    );
    assert_eq!(
        renderer.resolve("mapped/native", &from).unwrap(),
        Resolved::Empty
    );
    assert_eq!(
        renderer.resolve("mapped/obj", &from).unwrap(),
        file(&fixture, "node_modules/mapped/obj.js")
    );
    assert_eq!(
        main.resolve("nomanifest", &from).unwrap(),
        file(&fixture, "node_modules/nomanifest/index.js")
    );
    assert!(main.resolve("absent", &from).is_err());
}

#[test]
fn package_exports() {
    let fixture = Fixture::new(&[
        (
            "node_modules/str/package.json",
            "{\"exports\": \"./main.js\"}",
        ),
        ("node_modules/str/main.js", ""),
        (
            "node_modules/cond/package.json",
            "{\"exports\": {\"browser\": \"./b.js\", \"node\": {\"import\": \"./n.mjs\", \"require\": \"./n.cjs\"}, \"default\": \"./d.js\"}}",
        ),
        ("node_modules/cond/b.js", ""),
        ("node_modules/cond/n.mjs", ""),
        ("node_modules/cond/n.cjs", ""),
        ("node_modules/cond/d.js", ""),
        (
            "node_modules/map/package.json",
            r#"{"exports": {
                ".": {"types": "./x.d.ts", "default": "./index.js"},
                "./feature": "./src/feature.js",
                "./utils/*": "./src/utils/*.js",
                "./utils/special/*": [{"worker": "./w.js"}, "./src/special/*.js"],
                "./private/*": null,
                "./package.json": "./package.json"
            }}"#,
        ),
        ("node_modules/map/index.js", ""),
        ("node_modules/map/src/feature.js", ""),
        ("node_modules/map/src/utils/a.js", ""),
        ("node_modules/map/src/special/b.js", ""),
        ("src/x.js", ""),
    ]);
    let main = resolver(&fixture, Target::Main);
    let renderer = resolver(&fixture, Target::Renderer);
    let from = fixture.path("src/x.js");
    assert_eq!(
        main.resolve("str", &from).unwrap(),
        file(&fixture, "node_modules/str/main.js")
    );
    assert!(main.resolve("str/other", &from).is_err());
    assert_eq!(
        renderer.resolve("cond", &from).unwrap(),
        file(&fixture, "node_modules/cond/b.js")
    );
    assert_eq!(
        main.resolve("cond", &from).unwrap(),
        file(&fixture, "node_modules/cond/n.mjs")
    );
    assert_eq!(
        main.resolve("map", &from).unwrap(),
        file(&fixture, "node_modules/map/index.js")
    );
    assert_eq!(
        main.resolve("map/feature", &from).unwrap(),
        file(&fixture, "node_modules/map/src/feature.js")
    );
    assert_eq!(
        main.resolve("map/utils/a", &from).unwrap(),
        file(&fixture, "node_modules/map/src/utils/a.js")
    );
    assert_eq!(
        main.resolve("map/utils/special/b", &from).unwrap(),
        file(&fixture, "node_modules/map/src/special/b.js")
    );
    assert!(main.resolve("map/private/x", &from).is_err());
    assert!(main.resolve("map/nothing", &from).is_err());
    assert_eq!(
        main.resolve("map/package.json", &from).unwrap(),
        file(&fixture, "node_modules/map/package.json")
    );
}

#[test]
fn normalization() {
    assert_eq!(normalize(Path::new("a/./b/../c")), PathBuf::from("a/c"));
    assert_eq!(normalize(Path::new("../a")), PathBuf::from("../a"));
    assert_eq!(
        normalize(Path::new("../../a/../b")),
        PathBuf::from("../../b")
    );
    assert_eq!(normalize(Path::new("./a")), PathBuf::from("a"));
}

/// A package's request for its `.node` binary by relative path resolves to
/// the runtime's addon when the runtime provides that package's binary,
/// whether or not the binary was built; other binaries resolve as files.
#[test]
fn native_binaries_of_known_packages_are_runtime_addons() {
    let fixture = Fixture::new(&[
        ("node_modules/@vscode/windows-registry/dist/index.js", ""),
        ("node_modules/other/lib/index.js", ""),
        ("node_modules/other/build/Release/winregistry.node", ""),
    ]);
    let resolver = resolver(&fixture, Target::MainRequire);
    let registry = fixture.path("node_modules/@vscode/windows-registry/dist/index.js");
    assert_eq!(
        resolver.resolve("../build/Release/winregistry.node", &registry),
        Ok(Resolved::Builtin(
            "@vscode/windows-registry/winregistry.node".into()
        ))
    );
    let other = fixture.path("node_modules/other/lib/index.js");
    assert_eq!(
        resolver.resolve("../build/Release/winregistry.node", &other),
        Ok(file(
            &fixture,
            "node_modules/other/build/Release/winregistry.node"
        ))
    );
    assert_eq!(builtin("bindings"), Some("bindings".into()));
    assert_eq!(
        package_of(Path::new("a/node_modules/b/node_modules/c/x.js")).as_deref(),
        Some("c")
    );
    assert_eq!(package_of(Path::new("src/main.js")), None);
}
