use super::*;
use crate::tests::Fixture;

fn options(fixture: &Fixture, target: Target) -> BundleOptions {
    BundleOptions {
        root: fixture.root().to_path_buf(),
        target,
        aliases: Vec::new(),
        transpile: TranspileOptions::default(),
        emits: Vec::new(),
        mappings: Vec::new(),
    }
}

fn entry(fixture: &Fixture, path: &str) -> Entry {
    Entry {
        path: fixture.path(path),
        source: None,
    }
}

#[test]
fn keyboard_adapter_bypasses_installed_addon_and_missing_binary() {
    let fixture = Fixture::new(&[
        (
            "main.js",
            "const keyboard=require('native-keymap');module.exports=keyboard.getKeyMap();",
        ),
        (
            "node_modules/native-keymap/package.json",
            r#"{"main":"index.js","gypfile":true}"#,
        ),
        (
            "node_modules/native-keymap/index.js",
            "module.exports=require('./build/Release/keymapping.node')",
        ),
    ]);
    for target in [Target::Main, Target::Renderer] {
        let result = bundle(&[entry(&fixture, "main.js")], &options(&fixture, target)).unwrap();
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.modules.len(), 1);
        assert_eq!(
            result.modules[0].dependencies,
            vec![(
                "native-keymap".into(),
                Resolution::Builtin("native-keymap".into())
            )]
        );
    }
}

#[test]
fn browser_false_mappings_are_empty_dependencies() {
    let fixture = Fixture::new(&[
        (
            "main.js",
            "import './optional.js'; export const value = true;",
        ),
        ("package.json", r#"{"browser":{"./optional.js":false}}"#),
        ("optional.js", "throw new Error('must be replaced');"),
    ]);
    let result = bundle(
        &[entry(&fixture, "main.js")],
        &options(&fixture, Target::Renderer),
    )
    .unwrap();
    assert_eq!(result.modules.len(), 1);
    assert_eq!(
        result.modules[0].dependencies,
        vec![("./optional.js".into(), Resolution::Empty)]
    );
}

#[test]
fn prepared_frontend_modules_styles_and_assets_keep_portable_runtime_paths() {
    let fixture = Fixture::new(&[
        (
            "prepared/app.js",
            "import './theme.css';import icon from './icon.svg';import {value} from './chunk.js';export const result=[icon,value,import.meta.url];",
        ),
        ("prepared/chunk.js", "export const value=7;"),
        (
            "prepared/theme.css",
            "body{background-image:url('./icon.svg')}",
        ),
        ("prepared/icon.svg", "<svg/>"),
        ("unmapped.js", "export default true;"),
    ]);
    let mut settings = options(&fixture, Target::Renderer);
    settings.mappings.push(crate::PathMapping {
        source_root: fixture.path("prepared"),
        runtime_root: "out/renderer/assets".into(),
    });
    let found = bundle(
        &[
            entry(&fixture, "prepared/app.js"),
            entry(&fixture, "unmapped.js"),
        ],
        &settings,
    )
    .unwrap();
    assert_eq!(found.modules[0].path, "out/renderer/assets/app.js");
    assert_eq!(found.modules[1].path, "unmapped.js");
    assert!(
        found
            .modules
            .iter()
            .any(|module| module.path == "out/renderer/assets/chunk.js")
    );
    assert_eq!(found.styles[0].path, "out/renderer/assets/theme.css");
    assert!(
        found.styles[0]
            .source
            .contains("/out/renderer/assets/icon.svg")
    );
    assert_eq!(found.assets[0].url, "/out/renderer/assets/icon.svg");
    assert!(
        found
            .modules
            .iter()
            .all(|module| !Path::new(&module.path).is_absolute())
    );
}

fn module<'b>(bundle: &'b Bundle, path: &str) -> &'b Module {
    bundle
        .modules
        .iter()
        .find(|module| module.path == path)
        .unwrap_or_else(|| panic!("no module {path}"))
}

#[test]
fn external_workspace_packages_keep_portable_and_distinct_runtime_filenames() {
    let fixture = Fixture::new(&[
        (
            "packages/app/main.js",
            "require.resolve('shared'); require.resolve('./local');",
        ),
        ("packages/app/local.js", "module.exports = 1;"),
        ("node_modules/shared/index.js", "module.exports = 2;"),
    ]);
    let mut options = options(&fixture, Target::Main);
    options.root = fixture.path("packages/app");
    let result = bundle(&[entry(&fixture, "packages/app/main.js")], &options).unwrap();
    let external = result
        .modules
        .iter()
        .find(|module| module.path.ends_with("node_modules/shared/index.js"))
        .unwrap();
    assert!(Path::new(&external.path).is_absolute());
    let runtime = external.runtime_path.as_ref().unwrap();
    assert!(
        runtime.starts_with("node_modules/.nativeboat/"),
        "{runtime}"
    );
    assert!(runtime.ends_with("/shared/index.js"));
    assert!(!Path::new(runtime).is_absolute());
    assert!(!runtime.contains(".."));
    assert_eq!(module(&result, "main.js").runtime_path, None);
    assert_eq!(module(&result, "local.js").runtime_path, None);
}

#[test]
fn resolved_tools_outside_the_application_root_receive_portable_resource_paths() {
    let fixture = Fixture::new(&[
        (
            "app/main.js",
            "require.resolve('../tools/tool.exe'); require.resolve('./missing.node'); require.resolve('fs');",
        ),
        ("tools/tool.exe", "MZ shared executable"),
    ]);
    let mut options = options(&fixture, Target::Main);
    options.root = fixture.path("app");
    let result = bundle(&[entry(&fixture, "app/main.js")], &options).unwrap();
    assert_eq!(result.modules.len(), 1);
    let Resolution::File(path) = &result.modules[0].dependencies[0].1 else {
        panic!("resolve-only file");
    };
    assert!(path.starts_with(".nativeboat/files/"), "{path}");
    assert!(path.ends_with("/tool.exe"));
    assert!(Path::new(&result.assets[0].path).is_absolute());
    assert_eq!(result.assets[0].url, format!("/{path}"));
    assert!(matches!(
        result.modules[0].dependencies[1].1,
        Resolution::Missing(_)
    ));
    assert!(matches!(
        result.modules[0].dependencies[2].1,
        Resolution::Builtin(_)
    ));
}

fn project() -> Fixture {
    Fixture::new(&[
        (
            "src/main.tsx",
            "import React from 'react';\n\
             import { App } from './App';\n\
             import './style.css';\n\
             import logo from './logo.svg';\n\
             import raw from './logo.svg?raw';\n\
             import url from './notes.txt?url';\n\
             import data from './data.json';\n\
             import notes from './notes.txt';\n\
             import styles from './App.module.css';\n\
             import fs from 'node:fs';\n\
             let optional; try { optional = require('optional-missing'); } catch {}\n\
             const lazy = () => import('./App');\n\
             export const all = [React, App, logo, raw, url, data, notes, styles, fs, lazy, optional];\n",
        ),
        (
            "src/App.tsx",
            "import './style.css';\nexport const App = () => <div className=\"app\">hi</div>;\n",
        ),
        (
            "src/style.css",
            "@import './base.css';\n@import url(\"./base.css\");\n.app { background: url(./logo.svg) no-repeat; }\n\
             .b { background-image: url(\"./logo.svg?v=1#x\"); } .c { background: url(data:image/png;base64,AA) }\n\
             .d { background: url(#frag) } .e { background: url(http://x/y.png) } .f { background: url() }\n\
             .g { background: url(./missing.png) } .h { color: rgb(1 2 3) }\n",
        ),
        ("src/base.css", "body { margin: 0 }\n"),
        ("src/logo.svg", "<svg/>"),
        ("src/data.json", "{\"a\": 1}\n"),
        ("src/notes.txt", "hello \"world\""),
        (
            "src/App.module.css",
            ".title { color: red } :global(.keep) .title:hover { color: blue } .x :global(.y .z) .w {}\n\
             @media (max-width: 10px) { .title { color: green } }\n",
        ),
        (
            "node_modules/react/package.json",
            "{\"main\": \"index.js\"}",
        ),
        (
            "node_modules/react/index.js",
            "if (process.env.NODE_ENV === 'production') { module.exports = require('./cjs/react.production.js'); } \
             else { module.exports = require('./cjs/react.development.js'); }\n",
        ),
        (
            "node_modules/react/cjs/react.production.js",
            "exports.version = '18'; exports.jsx = () => null;",
        ),
        (
            "node_modules/react/jsx-runtime.js",
            "module.exports = require('./cjs/react.production.js');",
        ),
    ])
}

#[test]
fn linked_modules_share_identity_and_keep_their_dependency_versions_and_assets() {
    let fixture = Fixture::new(&[
        (
            "project/entry.js",
            "import a from 'linked-a'; import b from 'linked-b'; import old from 'versioned'; globalThis.values=[a,b,old];",
        ),
        (
            "project/node_modules/.pnpm/consumer/node_modules/consumer/package.json",
            r#"{"main":"index.js"}"#,
        ),
        (
            "project/node_modules/.pnpm/consumer/node_modules/consumer/index.js",
            "import './theme.css'; export {default} from 'versioned';",
        ),
        (
            "project/node_modules/.pnpm/consumer/node_modules/consumer/theme.css",
            ".consumer {background:url('./icon.svg')}",
        ),
        (
            "project/node_modules/.pnpm/consumer/node_modules/consumer/icon.svg",
            "<svg/>",
        ),
        (
            "project/node_modules/.pnpm/versioned/node_modules/versioned/index.js",
            "export default 'new';",
        ),
        (
            "project/node_modules/versioned/index.js",
            "export default 'old';",
        ),
        ("project/inline.js", ""),
    ]);
    fixture.junction(
        "project/node_modules/linked-a",
        "project/node_modules/.pnpm/consumer/node_modules/consumer",
    );
    fixture.junction(
        "project/node_modules/linked-b",
        "project/node_modules/.pnpm/consumer/node_modules/consumer",
    );
    fixture.junction(
        "project/node_modules/.pnpm/consumer/node_modules/versioned",
        "project/node_modules/.pnpm/versioned/node_modules/versioned",
    );
    fixture.junction("project-link", "project");
    let mut options = options(&fixture, Target::Renderer);
    options.root = fixture.path("project-link");
    let result = bundle_page(
        &[entry(
            &fixture,
            "project-link/node_modules/linked-a/theme.css",
        )],
        &[
            entry(&fixture, "project-link/entry.js"),
            Entry {
                path: fixture.path("project-link/inline.js"),
                source: Some("import a from 'linked-b';".to_owned()),
            },
            Entry {
                path: fixture.path("project-link/synthetic.js"),
                source: Some("import a from 'linked-a';".to_owned()),
            },
        ],
        &options,
    )
    .unwrap();
    let entry = module(&result, "entry.js");
    assert_eq!(entry.dependencies[0].1, entry.dependencies[1].1);
    assert_eq!(
        result
            .modules
            .iter()
            .filter(|module| module.path.ends_with("consumer/index.js"))
            .count(),
        1
    );
    assert!(
        module(&result, "node_modules/versioned/index.js")
            .code
            .contains("old")
    );
    assert!(
        module(
            &result,
            "node_modules/.pnpm/versioned/node_modules/versioned/index.js"
        )
        .code
        .contains("new")
    );
    assert!(
        result
            .modules
            .iter()
            .all(|module| !Path::new(&module.path).is_absolute())
    );
    assert_eq!(result.styles.len(), 1);
    assert_eq!(result.assets.len(), 1);
    assert_eq!(
        result.assets[0].path,
        "node_modules/.pnpm/consumer/node_modules/consumer/icon.svg"
    );
    assert!(result.warnings.is_empty());
}

#[test]
fn a_project_bundles_with_all_module_kinds() {
    let fixture = project();
    let bundle = bundle(
        &[entry(&fixture, "src/main.tsx")],
        &options(&fixture, Target::Renderer),
    )
    .unwrap();
    assert_eq!(bundle.entries, vec![0]);
    let paths: Vec<&str> = bundle
        .modules
        .iter()
        .map(|module| module.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "src/main.tsx",
            "node_modules/react/index.js",
            "src/App.tsx",
            "src/style.css",
            "src/logo.svg",
            "src/logo.svg",
            "src/notes.txt",
            "src/data.json",
            "src/notes.txt",
            "src/App.module.css",
            "node_modules/react/cjs/react.production.js",
            "node_modules/react/jsx-runtime.js",
        ]
    );
    let main = module(&bundle, "src/main.tsx");
    assert!(
        main.code
            .contains("__nui_interop(__nui_require(\"./App\"))")
    );
    assert_eq!(
        main.dependencies[0],
        ("react".to_owned(), Resolution::Module(1))
    );
    assert!(
        main.dependencies
            .contains(&("node:fs".to_owned(), Resolution::Builtin("fs".to_owned())))
    );
    assert!(main.dependencies.contains(&(
        "optional-missing".to_owned(),
        Resolution::Missing("optional-missing".to_owned())
    )));
    assert_eq!(
        bundle.modules[4].code,
        "module.exports = \"/src/logo.svg\";"
    );
    assert_eq!(bundle.modules[5].code, "module.exports = \"<svg/>\";");
    assert_eq!(
        bundle.modules[6].code,
        "module.exports = \"/src/notes.txt\";"
    );
    assert_eq!(bundle.modules[7].code, "module.exports = {\"a\": 1};");
    assert_eq!(
        bundle.modules[8].code,
        "module.exports = \"hello \\\"world\\\"\";"
    );
    let react = module(&bundle, "node_modules/react/index.js");
    assert_eq!(react.dependencies.len(), 1);
    let style_paths: Vec<&str> = bundle
        .styles
        .iter()
        .map(|style| style.path.as_str())
        .collect();
    assert_eq!(
        style_paths,
        vec!["src/base.css", "src/style.css", "src/App.module.css"]
    );
    let style = &bundle.styles[1].source;
    assert!(style.contains(".app { background: url(\"/src/logo.svg\") no-repeat; }"));
    assert!(style.contains("background-image: url(\"/src/logo.svg\");"));
    assert!(style.contains("url(data:image/png;base64,AA)"));
    assert!(style.contains("url(#frag)"));
    assert!(style.contains("url(http://x/y.png)"));
    assert!(style.contains("url(./missing.png)"));
    assert!(style.contains("rgb(1 2 3)"));
    assert_eq!(
        bundle.assets,
        vec![
            Asset {
                path: "src/logo.svg".to_owned(),
                url: "/src/logo.svg".to_owned()
            },
            Asset {
                path: "src/notes.txt".to_owned(),
                url: "/src/notes.txt".to_owned()
            },
        ]
    );
    assert!(
        bundle
            .warnings
            .iter()
            .any(|warning| warning.contains("optional-missing"))
    );
    assert!(
        bundle
            .warnings
            .iter()
            .any(|warning| warning.contains("url(./missing.png)"))
    );
}

#[test]
fn css_modules_scope_class_names() {
    let fixture = project();
    let bundle = bundle(
        &[entry(&fixture, "src/main.tsx")],
        &options(&fixture, Target::Renderer),
    )
    .unwrap();
    let suffix = scope_suffix("src/App.module.css");
    let css = &bundle.styles[2].source;
    assert!(css.contains(&format!(".title_{suffix} {{ color: red }}")));
    assert!(css.contains(&format!(".keep .title_{suffix}:hover")));
    assert!(css.contains(&format!(".x_{suffix} .y .z .w_{suffix}")));
    assert!(css.contains(&format!("@media (max-width: 10px) {{ .title_{suffix} ")));
    let code = &module(&bundle, "src/App.module.css").code;
    assert_eq!(
        code,
        &format!(
            "module.exports = {{\"title\": \"title_{suffix}\", \"w\": \"w_{suffix}\", \"x\": \"x_{suffix}\"}};"
        )
    );
    assert_eq!(
        module(&bundle, "src/style.css").code,
        "module.exports = {};"
    );
    assert_eq!(scope_suffix("a").len(), 6);
}

#[test]
fn inline_entries_and_outside_paths() {
    let fixture = Fixture::new(&[("src/a.js", "module.exports = 1;")]);
    let outside = Fixture::new(&[("shared.js", "module.exports = 2;")]);
    let shared = outside
        .path("shared.js")
        .to_string_lossy()
        .replace('\\', "/");
    let bundle = super::bundle(
        &[Entry {
            path: fixture.path("inline.js"),
            source: Some(format!(
                "import a from './src/a.js'; import b from '{shared}'; console.log(a, b);"
            )),
        }],
        &options(&fixture, Target::Renderer),
    )
    .unwrap();
    assert_eq!(bundle.modules[0].path, "inline.js");
    assert!(bundle.modules[2].path.ends_with("shared.js"));
    assert!(Path::new(&bundle.modules[2].path).is_absolute());
}

#[test]
fn compiled_sources_know_their_output_paths() {
    let fixture = Fixture::new(&[
        (
            "src/main/main.ts",
            "import { x } from './util.js'; import y from 'dep'; console.log(x, y);",
        ),
        ("src/main/util.ts", "export const x = 1;"),
        ("node_modules/dep/index.js", "module.exports = 2;"),
    ]);
    let mut options = options(&fixture, Target::Main);
    options.emits = vec![crate::Emit {
        root_dir: String::new(),
        out_dir: "dist".to_owned(),
    }];
    // Entries name compiled files, as pages do: `./main.js` is `main.ts`.
    let bundle = super::bundle(&[entry(&fixture, "src/main/./main.js")], &options).unwrap();
    let runtime = |path: &str| module(&bundle, path).runtime_path.clone();
    assert_eq!(
        runtime("src/main/main.ts").as_deref(),
        Some("dist/src/main/main.js")
    );
    assert_eq!(
        runtime("src/main/util.ts").as_deref(),
        Some("dist/src/main/util.js")
    );
    assert_eq!(
        runtime("node_modules/dep/index.js").as_deref(),
        Some("node_modules/dep/index.js")
    );
}

#[test]
fn failures_are_collected() {
    let fixture = Fixture::new(&[
        (
            "src/main.js",
            "import './missing';\nimport bad from './bad.json';\nimport addon from './addon.node';\n\
             import './broken.js';\nimport './bad-import.css';\nimport './builtin.css';\nimport './nested.css';\n",
        ),
        ("src/bad.json", "{ nope"),
        ("src/addon.node", ""),
        ("src/broken.js", "const = ;"),
        ("src/bad-import.css", "@import './nowhere.css';"),
        ("src/builtin.css", "@import 'fs';"),
        ("src/nested.css", "@import './binary.css';"),
    ]);
    fixture.write("src/binary.css", &[0xff, 0xfe, 0x00]);
    let errors = bundle(
        &[entry(&fixture, "src/main.js")],
        &options(&fixture, Target::Main),
    )
    .unwrap_err();
    assert!(
        errors[0].starts_with("cannot resolve \"./missing\""),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.starts_with("src/bad.json: invalid JSON"))
    );
    assert!(errors.contains(&"src/addon.node: native Node.js addons are not supported".to_owned()));
    assert!(
        errors
            .iter()
            .any(|error| error.starts_with("src/broken.js: "))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.starts_with("cannot resolve \"./nowhere.css\""))
    );
    assert!(errors.contains(&"cannot import module \"fs\" from CSS".to_owned()));
    assert!(
        errors
            .iter()
            .any(|error| error.starts_with("src/binary.css: "))
    );
    let errors = bundle(
        &[entry(&fixture, "src/absent.js")],
        &options(&fixture, Target::Main),
    )
    .unwrap_err();
    assert!(errors[0].starts_with("src/absent.js: "));
}

#[test]
fn dynamic_requires_are_warnings_and_modules_are_shared() {
    let fixture = Fixture::new(&[
        (
            "main.js",
            "const name = 'x';\nrequire(name);\nrequire('./shared');\nrequire('./other');\n",
        ),
        ("shared.js", "module.exports = 1;"),
        ("other.js", "require('./shared');"),
    ]);
    let bundle = bundle(
        &[entry(&fixture, "main.js")],
        &options(&fixture, Target::Main),
    )
    .unwrap();
    assert_eq!(bundle.modules.len(), 3);
    assert_eq!(
        bundle.warnings,
        vec!["main.js: dynamic require: require(name)".to_owned()]
    );
    let dep = Fixture::new(&[(
        "node_modules/dep/index.js",
        "const a = 1 < 2 > 0;\nmodule.exports = a;",
    )]);
    let bundle = super::bundle(
        &[entry(&dep, "node_modules/dep/index.js")],
        &options(&dep, Target::Main),
    )
    .unwrap();
    assert!(bundle.modules[0].code.contains("1 < 2 > 0"));
}

#[test]
fn kinds_and_strings() {
    assert_eq!(kind_of(Path::new("a.cjs"), ""), Kind::Script);
    assert_eq!(kind_of(Path::new("a.MTS"), ""), Kind::Script);
    assert_eq!(kind_of(Path::new("a.html"), ""), Kind::Raw);
    assert_eq!(kind_of(Path::new("a.md"), ""), Kind::Raw);
    assert_eq!(kind_of(Path::new("a.woff2"), ""), Kind::Asset);
    assert_eq!(kind_of(Path::new("noextension"), ""), Kind::Asset);
    assert_eq!(kind_of(Path::new(""), ""), Kind::Asset);
    assert_eq!(kind_of(Path::new("a.js"), "url"), Kind::Asset);
    assert_eq!(kind_of(Path::new("a.js"), "raw"), Kind::Raw);
    assert_eq!(kind_of(Path::new("a.js"), "inline"), Kind::Script);
    assert_eq!(js_string("a\"b\n"), "\"a\\\"b\\n\"");
}

#[test]
fn bundles_serialize() {
    let fixture = project();
    let bundle = bundle(
        &[entry(&fixture, "src/main.tsx")],
        &options(&fixture, Target::Renderer),
    )
    .unwrap();
    let json = serde_json::to_string(&bundle).unwrap();
    assert_eq!(serde_json::from_str::<Bundle>(&json).unwrap(), bundle);
}

#[test]
fn pages_bundle_styles_before_scripts() {
    let fixture = Fixture::new(&[
        ("index.html", ""),
        (
            "linked.css",
            "@import './base.css';
.linked { background: url(./logo.png) }",
        ),
        ("base.css", "body { margin: 0 }"),
        ("logo.png", "png"),
        ("main.js", "import './imported.css';"),
        ("imported.css", ".imported {}"),
    ]);
    let styles = [
        entry(&fixture, "linked.css"),
        Entry {
            path: fixture.path("index.html.style-0.css"),
            source: Some(".inline { color: red }".to_owned()),
        },
    ];
    let bundle = bundle_page(
        &styles,
        &[entry(&fixture, "main.js")],
        &options(&fixture, Target::Renderer),
    )
    .unwrap();
    let paths: Vec<&str> = bundle
        .styles
        .iter()
        .map(|style| style.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "base.css",
            "linked.css",
            "index.html.style-0.css",
            "imported.css"
        ]
    );
    assert_eq!(bundle.entries, vec![0]);
    assert_eq!(bundle.assets[0].url, "/logo.png");
    let errors = bundle_page(
        &[entry(&fixture, "missing.css")],
        &[],
        &options(&fixture, Target::Renderer),
    )
    .unwrap_err();
    assert!(errors[0].starts_with("missing.css: "));
}
