use super::*;
use crate::tests::fixture::Fixture;

#[test]
fn two_package_layouts_use_production_metadata_and_root_build_configuration() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"tooling","version":"1.0.0","description":"fallback","main":"main.js","devDependencies":{"electron":"44"},"build":{"productName":"Shown","directories":{"app":"release/app"},"publish":"github"},"repository":"github:owner/repo"}"#,
        ),
        ("main.js", "require('electron');"),
        (
            "release/app/package.json",
            r#"{"name":"application","version":"4.6.0"}"#,
        ),
    ]);
    let project = crate::load(fixture.root()).unwrap();
    assert_eq!(project.metadata.name, "application");
    assert_eq!(project.metadata.version, "4.6.0");
    assert_eq!(project.metadata.product_name, "Shown");
    assert_eq!(project.metadata.description, "fallback");
    assert_eq!(project.main.source, fixture.path("main.js"));
    assert_eq!(
        project.updater_config.unwrap()["updaterCacheDirName"],
        "application-updater"
    );
    // Feeds of Electron builds are flagged until the native build has its own.
    assert!(
        project
            .warnings
            .iter()
            .any(|warning| warning.contains("\"nativeboat\": {\"publish\": ...}"))
    );
    std::fs::write(
        fixture.path("package.json"),
        r#"{"name":"tooling","main":"main.js","devDependencies":{"electron":"44"},"build":{"publish":"github"},"repository":"github:owner/repo","nativeboat":{"publish":{"provider":"generic","url":"https://native.example"}}}"#,
    )
    .unwrap();
    let project = crate::load(fixture.root()).unwrap();
    assert_eq!(
        project.updater_config.unwrap()["url"],
        "https://native.example"
    );
    assert!(
        !project
            .warnings
            .iter()
            .any(|warning| warning.contains("auto-updates"))
    );
    // A settings file gives the native feed without changing package.json.
    std::fs::write(
        fixture.path("package.json"),
        r#"{"name":"tooling","main":"main.js","devDependencies":{"electron":"44"},"build":{"publish":"github"},"repository":"github:owner/repo"}"#,
    )
    .unwrap();
    let settings = crate::settings::Settings {
        publish: Some(serde_json::json!({"provider":"generic","url":"https://settings.example"})),
        plugins: None,
        documents: None,
        forks: None,
        unknown: vec!["later".to_owned()],
    };
    let project = crate::load_with(fixture.root(), &settings).unwrap();
    assert_eq!(
        project.updater_config.unwrap()["url"],
        "https://settings.example"
    );
    assert!(
        !project
            .warnings
            .iter()
            .any(|warning| warning.contains("auto-updates"))
    );
    assert!(
        project
            .warnings
            .iter()
            .any(|warning| warning.contains("\"later\", which this NativeBoat does not use"))
    );
    let package = serde_json::json!({"name":"root"});
    assert_eq!(
        application_package(fixture.root(), &serde_json::json!({}), &package),
        package
    );
    assert_eq!(
        application_package(
            fixture.root(),
            &serde_json::json!({"directories":{"app":"missing"}}),
            &package
        ),
        package
    );
    fixture.write("release/app/package.json", b"null");
    assert_eq!(
        application_package(
            fixture.root(),
            &serde_json::json!({"directories":{"app":"release/app"}}),
            &package
        ),
        package
    );
    fixture.write("release/app/package.json", b"{}");
    assert_eq!(
        application_package(
            fixture.root(),
            &serde_json::json!({"directories":{"app":"release/app"}}),
            &Value::Null
        ),
        Value::Null
    );
}

fn copies(files: &[ExtraFile], root: &Path) -> Vec<(String, String)> {
    files
        .iter()
        .map(|file| {
            (
                crate::relative(root, &file.source),
                file.destination.clone(),
            )
        })
        .collect()
}

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(source, destination)| ((*source).to_owned(), (*destination).to_owned()))
        .collect()
}

#[test]
fn configuration_comes_from_a_file_or_package_json() {
    let fixture = Fixture::new(&[]);
    let package = serde_json::json!({"build": {"appId": "a"}});
    assert_eq!(config(fixture.root(), &package).unwrap()["appId"], "a");
    fixture.write(
        "electron-builder.json5",
        b"{appId: 'b', // JSON5\n win: {sign: false}, number: 0x10, publish: null,}",
    );
    assert_eq!(config(fixture.root(), &package).unwrap()["appId"], "a");
    let package = serde_json::json!({});
    let build = config(fixture.root(), &package).unwrap();
    assert_eq!(build["appId"], "b");
    assert_eq!(build["win"]["sign"], false);
    assert_eq!(build["number"], 16);
    assert!(build["publish"].is_null());
    fixture.write("electron-builder.json", br#"{"appId": "c", /* file */}"#);
    assert_eq!(config(fixture.root(), &package).unwrap()["appId"], "c");
    fixture.write("electron-builder.yaml", b"appId: d\n");
    assert_eq!(config(fixture.root(), &package).unwrap()["appId"], "d");
    fixture.write(
        "electron-builder.yml",
        b"defaults: &defaults {appId: e, productName: Default}\n<<: *defaults\nproductName: Override\n",
    );
    let build = config(fixture.root(), &package).unwrap();
    assert_eq!(build["appId"], "e");
    assert_eq!(build["productName"], "Override");
    assert!(build.get("<<").is_none());
    assert!(config(fixture.root(), &serde_json::json!({"build":false})).is_err());
}

#[test]
fn invalid_configuration_is_reported_without_falling_back_to_another_file() {
    for name in [
        "electron-builder.yml",
        "electron-builder.yaml",
        "electron-builder.json",
        "electron-builder.json5",
    ] {
        let fixture = Fixture::new(&[(name, "{"), ("electron-builder.json5", "{}")]);
        fixture.write(name, b"{");
        let package = serde_json::json!({});
        let error = config(fixture.root(), &package).unwrap_err();
        assert!(
            error.contains(name) && error.contains("invalid "),
            "{error}"
        );
        fixture.write(name, b"[]");
        assert!(
            config(fixture.root(), &package)
                .unwrap_err()
                .contains("build configuration must be an object")
        );
        std::fs::remove_file(fixture.path(name)).unwrap();
        std::fs::create_dir(fixture.path(name)).unwrap();
        assert!(
            config(fixture.root(), &package)
                .unwrap_err()
                .contains("cannot read ")
        );
    }
    let fixture = Fixture::new(&[]);
    assert_eq!(
        config(fixture.root(), &serde_json::json!({})).unwrap(),
        Value::Null
    );
}

#[test]
fn yaml_packaging_settings_supply_identity_resources_and_output_paths() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"app","version":"1.2.3","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        ("main.js", "require('electron')"),
        (
            "electron-builder.yml",
            "appId: com.example.app\nproductName: Desktop\nwin:\n  icon: static/icon.ico\ndirectories:\n  output: ../../dist\nextraResources:\n  - from: static\n    to: assets\n    filter: ['**/*', '!ignored.txt']\npublish: null\n",
        ),
        ("static/icon.ico", "icon"),
        ("static/nested/theme.css", "body {color: red}"),
        ("static/ignored.txt", "ignored"),
    ]);
    let project = crate::load(fixture.root()).unwrap();
    assert_eq!(project.metadata.product_name, "Desktop");
    assert_eq!(project.metadata.app_id.as_deref(), Some("com.example.app"));
    assert_eq!(project.metadata.icon, Some(fixture.path("static/icon.ico")));
    assert_eq!(project.output_directory, "../../dist");
    assert_eq!(
        copies(&project.extra_files, fixture.root()),
        owned(&[
            ("static/icon.ico", "resources/assets/icon.ico"),
            (
                "static/nested/theme.css",
                "resources/assets/nested/theme.css"
            ),
        ])
    );
    assert!(project.updater_config.is_none());
}

#[test]
fn executables_go_to_the_output_directory() {
    let emit = |out_dir: &str| Emit {
        root_dir: String::new(),
        out_dir: out_dir.to_owned(),
    };
    let configured = serde_json::json!({"directories": {"output": "release-builds"}});
    assert_eq!(
        output_directory(&configured, &[emit("dist")]),
        "release-builds"
    );
    let none = serde_json::Value::Null;
    assert_eq!(output_directory(&none, &[]), "dist");
    assert_eq!(output_directory(&none, &[emit("out")]), "dist");
    assert_eq!(output_directory(&none, &[emit("dist/electron")]), "release");
}

#[test]
fn extra_resources_and_files_are_collected() {
    let fixture = Fixture::new(&[
        ("sidecars/server.exe", "exe"),
        ("sidecars/server.pdb", "pdb"),
        ("sidecars/lib/helper.dll", "dll"),
        ("assets/icon.png", "png"),
        ("assets/node_modules/x/index.js", "js"),
        ("README.md", "readme"),
        ("licenses/LICENSE.txt", "license"),
        ("bin/tool.exe", "tool"),
    ]);
    let config = serde_json::json!({
        "extraResources": [
            {"from": "sidecars", "to": "sidecars", "filter": ["**/*", "!*.pdb"]},
            "assets/**/*.png",
            {"from": "README.md"},
            {"from": "README.md", "to": "docs/readme.md"},
            {"from": "./sidecars", "to": "dot", "filter": "server.exe"},
            {"from": "missing", "to": "x"},
            {"from": "assets", "filter": ["sub/core.exe", "*.dll"]},
            {"from": "gone", "filter": "server.exe"},
            42
        ],
        "extraFiles": {"from": "licenses", "filter": "*.txt"},
        "win": {
            "extraResources": {"from": "sidecars/lib"},
            "extraFiles": [{"from": "bin", "to": "tools"}]
        }
    });
    let mut warnings = Vec::new();
    let mut missing = Vec::new();
    let files = extra_files(fixture.root(), &config, &mut warnings, &mut missing);
    assert_eq!(
        copies(&files, fixture.root()),
        owned(&[
            ("assets/icon.png", "resources/assets/icon.png"),
            (
                "sidecars/lib/helper.dll",
                "resources/sidecars/lib/helper.dll"
            ),
            ("sidecars/server.exe", "resources/sidecars/server.exe"),
            ("README.md", "resources/README.md"),
            ("README.md", "resources/docs/readme.md"),
            ("sidecars/server.exe", "resources/dot/server.exe"),
            ("sidecars/lib/helper.dll", "resources/helper.dll"),
            ("licenses/LICENSE.txt", "LICENSE.txt"),
            ("bin/tool.exe", "tools/tool.exe"),
        ])
    );
    assert_eq!(
        warnings,
        vec![
            "extraResources \"missing\" does not exist and is not copied; build it before compiling",
            "extraResources \"assets\" has no files to copy; build them before compiling",
            "extraResources \"gone\" does not exist and is not copied; build it before compiling",
        ]
    );
    // What is missing, for finding the scripts that build it.
    assert_eq!(missing, vec!["missing", "core.exe", "gone", "server.exe"]);
    assert!(
        extra_files(
            fixture.root(),
            &serde_json::Value::Null,
            &mut warnings,
            &mut missing
        )
        .is_empty()
    );
}

#[test]
fn resource_filters_preserve_order_groups_and_platform_exclusions() {
    let fixture = Fixture::new(&[
        ("static/locales/en.json", "source"),
        ("static/locales/en.min.json", "generated"),
        ("static/locales/nested/de.json", "source"),
        ("static/locales/nested/de.min.json", "generated"),
        ("static/preference.json", "settings"),
        ("static/.hidden", "hidden"),
        ("static/keep.txt", "keep"),
        ("static/drop.txt", "drop"),
        ("themes/light/theme.css", "theme"),
    ]);
    let config = serde_json::json!({
        "extraResources": [
            "static/**/*", "!static/locales/**/!(*.min).json", "!static/*.txt",
            {"from":"static", "to":"mapped", "filter":["**/*", "!**/*.json", "locales/**/@(en|de).min.json"]}
        ],
        "win":{"extraResources":["static/keep.txt", "!static/drop.txt"]},
        "extraFiles":["themes", "!themes/**/*.map"]
    });
    let mut warnings = Vec::new();
    let mut missing = Vec::new();
    let files = extra_files(fixture.root(), &config, &mut warnings, &mut missing);
    assert_eq!(
        copies(&files, fixture.root()),
        owned(&[
            ("static/.hidden", "resources/static/.hidden"),
            ("static/keep.txt", "resources/static/keep.txt"),
            (
                "static/locales/en.min.json",
                "resources/static/locales/en.min.json"
            ),
            (
                "static/locales/nested/de.min.json",
                "resources/static/locales/nested/de.min.json"
            ),
            ("static/preference.json", "resources/static/preference.json"),
            ("static/.hidden", "resources/mapped/.hidden"),
            ("static/drop.txt", "resources/mapped/drop.txt"),
            ("static/keep.txt", "resources/mapped/keep.txt"),
            (
                "static/locales/en.min.json",
                "resources/mapped/locales/en.min.json"
            ),
            (
                "static/locales/nested/de.min.json",
                "resources/mapped/locales/nested/de.min.json"
            ),
            ("themes/light/theme.css", "themes/light/theme.css"),
        ])
    );
    assert!(warnings.is_empty() && missing.is_empty());
    for (filters, path, expected) in [
        (&[][..], "any/file", true),
        (&["!**/*.map"][..], "src/main.js", true),
        (&["!**/*.map"][..], "src/main.js.map", false),
        (&["**/*", "!*.txt", "keep.txt"][..], "keep.txt", true),
        (&["keep.txt", "!*.txt"][..], "keep.txt", false),
        (&["!!*.txt"][..], "keep.txt", true),
        (&["**/*", "!!!*.txt"][..], "keep.txt", false),
    ] {
        let filters: Vec<String> = filters.iter().map(|filter| (*filter).to_owned()).collect();
        assert_eq!(selected(&filters, path), expected, "{filters:?}: {path}");
    }
    assert!(literal_names(&["@(en|de).json".to_owned(), "+(a).json".to_owned()]).is_empty());
}

#[test]
fn script_configurations_supply_their_static_settings() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"scripted","main":"main.js","devDependencies":{"electron":"44"}}"#,
        ),
        ("main.js", "require('electron');"),
        ("public/icon.png", "png"),
        (
            "electron-builder.config.ts",
            "import type { Configuration } from 'electron-builder'
const notarize = process.env.SKIP_NOTARIZE !== 'true'
const config: Configuration = {
  appId: 'com.example.scripted',
  productName: 'Scripted',
  afterSign: notarize ? 'scripts/notarize.js' : undefined,
  extraResources: [{ from: 'public', to: 'public' }],
}
export default config",
        ),
    ]);
    let package: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture.path("package.json")).unwrap())
            .unwrap();
    let build = config(fixture.root(), &package).unwrap();
    assert_eq!(build["appId"], "com.example.scripted");
    assert_eq!(build["productName"], "Scripted");
    // What depends on running the module is unknown.
    assert_eq!(build["afterSign"], Value::Null);
    let project = crate::load(fixture.root()).unwrap();
    assert_eq!(project.metadata.product_name, "Scripted");
    assert_eq!(
        copies(&project.extra_files, fixture.root()),
        owned(&[("public/icon.png", "resources/public/icon.png")])
    );
}

fn destinations(files: &[ExtraFile]) -> Vec<&str> {
    files.iter().map(|file| file.destination.as_str()).collect()
}

#[test]
fn application_files_follow_the_files_patterns_without_compiled_sources() {
    let fixture = Fixture::new(&[
        ("package.json", "{}"),
        ("main.js", ""),
        ("preload.js", ""),
        (
            "html/index.html",
            "<link rel=stylesheet href=../assets/a.css>",
        ),
        ("assets/a.css", "body{}"),
        ("assets/images/logo.png", "png"),
        ("assets/lib.js", ""),
        ("src/renderer.js", ""),
        ("src/types.d.ts", ""),
        ("resources/icon.ico", "ico"),
        ("ffmpeg/bin/ffmpeg.exe", "exe"),
        ("ffmpeg.patch", "patch"),
        ("node.exe", "exe"),
        ("node_modules/pkg/data.json", "{}"),
        ("release/YTDownloader.exe", "built"),
        ("notes.md", "not selected"),
        ("translations/en.json", "{}"),
        ("tsconfig.json", "{}"),
        ("tsconfig.node.json", "{}"),
        ("electron-builder.yml", ""),
    ]);
    let config = serde_json::json!({
        "directories": {"buildResources": "resources", "output": "release"},
        "files": [
            "./resources/**/*", "main.js", "./html/**/*", "./assets/**/*", "./src/*.js",
            "./ffmpeg/**/*", "!ffmpeg.patch", "translations", "node*", "package.json", "*.json", "*.yml"
        ]
    });
    let files = application_files(fixture.root(), &config, &[PathBuf::from("release")]);
    assert_eq!(
        destinations(&files),
        [
            "assets/a.css",
            "assets/images/logo.png",
            "ffmpeg/bin/ffmpeg.exe",
            "html/index.html",
            "node.exe",
            "package.json",
            "resources/icon.ico",
            "translations/en.json",
        ]
    );
    assert_eq!(files[0].source, fixture.path("assets/a.css"));
}

#[test]
fn installed_dependencies_ship_their_files_beside_the_executable() {
    // The packages the application and its nested packages depend on when
    // they run, found as Node finds them, ship the files they read, at their
    // places in the folder; code, native addons, build leftovers, other
    // systems' libraries, documentation, tests and dev dependencies do not.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"dependencies": {"a": "1"}, "devDependencies": {"dev": "1"}}"#,
        ),
        (
            "node_modules/a/package.json",
            r#"{"dependencies": {"b": "1"}}"#,
        ),
        ("node_modules/a/index.js", ""),
        ("node_modules/a/data.json", "{}"),
        ("node_modules/a/build/Release/addon.node", "addon"),
        ("node_modules/a/src/addon.h", ""),
        ("node_modules/a/README.md", ""),
        ("node_modules/a/test/fixture.json", "{}"),
        ("node_modules/a/node_modules/b/package.json", "{}"),
        ("node_modules/a/node_modules/b/b.wasm", "wasm"),
        ("node_modules/a/node_modules/b/bin/linux/b.so", "so"),
        ("node_modules/dev/package.json", "{}"),
        ("node_modules/dev/dev.json", "{}"),
        (
            "plugins/p/package.json",
            r#"{"optionalDependencies": {"c": "1"}}"#,
        ),
        ("plugins/p/node_modules/c/package.json", "{}"),
        ("plugins/p/node_modules/c/bin/c.exe", "exe"),
    ]);
    let files = application_files(fixture.root(), &serde_json::json!({}), &[]);
    assert_eq!(
        destinations(&files),
        [
            "node_modules/a/data.json",
            "node_modules/a/node_modules/b/b.wasm",
            "node_modules/a/node_modules/b/package.json",
            "node_modules/a/package.json",
            "package.json",
            "plugins/p/node_modules/c/bin/c.exe",
            "plugins/p/node_modules/c/package.json",
            "plugins/p/package.json",
        ]
    );
}

#[test]
fn exclusion_only_and_missing_patterns_take_everything_but_build_resources() {
    let fixture = Fixture::new(&[
        ("package.json", "{}"),
        ("build/icons/icon.ico", "ico"),
        ("static/logo.png", "png"),
        ("src/main/index.ts", ""),
        ("src/renderer/style.css", "p{}"),
        ("docs/guide.md", ""),
        ("README.md", ""),
        ("CHANGELOG.md", ""),
        (".github/workflow.yml", ""),
        ("pnpm-lock.yaml", ""),
        ("dist/app.exe", ""),
    ]);
    let config = serde_json::json!({
        "files": ["!src/*", "!docs/*", "!{README.md,CHANGELOG.md}"]
    });
    let files = application_files(fixture.root(), &config, &[PathBuf::from("dist")]);
    assert_eq!(destinations(&files), ["package.json", "static/logo.png"]);
    let everything = application_files(fixture.root(), &serde_json::json!({}), &[]);
    assert_eq!(
        destinations(&everything),
        [
            "CHANGELOG.md",
            "README.md",
            "dist/app.exe",
            "docs/guide.md",
            "package.json",
            "src/renderer/style.css",
            "static/logo.png",
        ]
    );
    // A set mapped elsewhere keeps its destination; a missing one is skipped.
    let mapped = serde_json::json!({
        "files": [{"from": "static", "to": "public", "filter": ["**/*"]}, {"from": "absent"}]
    });
    assert_eq!(
        destinations(&application_files(fixture.root(), &mapped, &[])),
        ["public/logo.png"]
    );
}

#[test]
fn brace_groups_expand_into_their_alternatives() {
    assert_eq!(braces("plain"), ["plain"]);
    assert_eq!(braces("{a,b}.md"), ["a.md", "b.md"]);
    assert_eq!(
        braces("x/{a,{b,c}}/{1,2}"),
        ["x/a/1", "x/a/2", "x/b/1", "x/b/2", "x/c/1", "x/c/2"]
    );
    assert_eq!(
        braces("**/typescript{,/**}"),
        ["**/typescript", "**/typescript/**"]
    );
    assert_eq!(braces("{single}.md"), ["{single}.md"]);
    assert_eq!(braces("{open"), ["{open"]);
}

#[test]
fn excluded_directories_are_pruned_unless_included_again() {
    let filters = ["!src/*".to_owned(), "src/keep/**/*".to_owned()];
    assert!(pruned(&filters, "src/drop"));
    assert!(!pruned(&filters, "src/keep"));
    assert!(!pruned(&filters, "other"));
    assert!(pruned(&["!{a,b}".to_owned()], "b"));
}
