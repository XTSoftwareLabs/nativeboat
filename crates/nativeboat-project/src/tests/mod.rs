pub(crate) mod fixture;
mod marktext;

use super::*;
use fixture::Fixture;

fn paths(entries: &[Entry], root: &Path) -> Vec<(String, Vec<String>)> {
    entries
        .iter()
        .map(|entry| (relative(root, &entry.source), entry.runtime_paths.clone()))
        .collect()
}

fn owned(items: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    items
        .iter()
        .map(|(source, runtime)| {
            (
                (*source).to_owned(),
                runtime.iter().map(|path| (*path).to_owned()).collect(),
            )
        })
        .collect()
}

#[test]
fn plain_electron_projects() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name": "plain-app", "version": "1.2.3", "description": "Demo", "main": "./main.js",
                "dependencies": {"left-pad": "1"}, "devDependencies": {"electron": "^30.0.0"},
                "optionalDependencies": {"opt": 2},
                "build": {"appId": "com.example.plain", "win": {"icon": "build/icon.ico"},
                          "electronLanguages": ["en-US", "fr"]}}"#,
        ),
        (
            "main.js",
            "const { app, BrowserWindow } = require('electron');\nconst path = require('path');\n\
             const win = new BrowserWindow({ webPreferences: { preload: path.join(__dirname, 'preload.js') } });\n\
             win.loadFile('index.html');\nwin.loadFile(path.join(__dirname, 'pages/about.html'));\n\
             win.loadFile('missing.html');\nwin.loadFile('./index.html');\n\
             const other = { preload: 'nowhere.js' };\nconst again = { preload: path.join(__dirname, 'preload') };\n",
        ),
        ("preload.js", "require('electron');"),
        ("index.html", "<html></html>"),
        ("pages/about.html", "<html></html>"),
        ("build/icon.ico", "ico"),
        (
            ".env",
            "VITE_TITLE=Hello\nSECRET=x\nMAIN_VITE_PORT=1\nRENDERER_VITE_THEME=dark",
        ),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.layout, Layout::Plain);
    assert_eq!(project.metadata.name, "plain-app");
    assert_eq!(project.metadata.product_name, "plain-app");
    assert_eq!(project.metadata.version, "1.2.3");
    assert_eq!(project.metadata.description, "Demo");
    assert_eq!(
        project.metadata.app_id.as_deref(),
        Some("com.example.plain")
    );
    assert_eq!(project.metadata.icon, Some(fixture.path("build/icon.ico")));
    assert_eq!(project.metadata.locales, ["en-US", "fr"]);
    assert!(project.depends_on("left-pad"));
    assert!(project.depends_on("opt"));
    assert_eq!(project.dependencies["opt"], "");
    assert!(!project.depends_on("react"));
    assert_eq!(
        paths(std::slice::from_ref(&project.main), fixture.root()),
        owned(&[("main.js", &["main.js"])])
    );
    assert_eq!(
        paths(&project.preloads, fixture.root()),
        owned(&[("preload.js", &["preload.js", "preload"])])
    );
    assert_eq!(
        paths(&project.pages, fixture.root()),
        owned(&[
            ("index.html", &["index.html"]),
            ("pages/about.html", &["pages/about.html"])
        ])
    );
    assert_eq!(project.renderer_env["VITE_TITLE"], "Hello");
    assert_eq!(project.renderer_env["RENDERER_VITE_THEME"], "dark");
    assert!(!project.renderer_env.contains_key("SECRET"));
    assert!(!project.renderer_env.contains_key("MAIN_VITE_PORT"));
    assert_eq!(project.main_env["MAIN_VITE_PORT"], "1");
    assert_eq!(project.preload_env["PROD"], true);
    assert_eq!(project.renderer_env["MODE"], "production");
    assert!(project.warnings.is_empty());
    assert!(project.defines.is_empty());
    assert_eq!(project.output_directory, "dist");
}

#[test]
fn plain_projects_fall_back_to_index_html() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"productName": "Shown", "dependencies": {"electron-nightly": "1"}, "build": {"icon": "missing.png", "electronLanguages": "de",
                "win": {"electronLanguages": "zh-CN"}}}"#,
        ),
        ("index.js", "console.log('no windows');"),
        ("src/index.html", "<html></html>"),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.metadata.name, "app");
    assert_eq!(project.metadata.product_name, "Shown");
    assert_eq!(project.metadata.app_name, "Shown");
    assert_eq!(project.metadata.version, "0.0.0");
    assert_eq!(project.metadata.icon, None);
    // Windows' own setting comes first.
    assert_eq!(project.metadata.locales, ["zh-CN"]);
    assert_eq!(project.main.runtime_paths, vec!["index.js".to_owned()]);
    assert_eq!(
        paths(&project.pages, fixture.root()),
        owned(&[("src/index.html", &["src/index.html"])])
    );
    let headless = Fixture::new(&[
        (
            "package.json",
            r#"{"build": {"productName": "Tray"}, "dependencies": {"electron": "1"}}"#,
        ),
        ("index.js", "const { Tray } = require('electron');"),
    ]);
    let project = load(headless.root()).unwrap();
    assert_eq!(project.metadata.product_name, "Tray");
    assert!(project.metadata.locales.is_empty());
    // Electron names the running app from package.json alone.
    assert_eq!(project.metadata.app_name, "app");
    // An application without windows has no pages to miss.
    assert!(project.warnings.is_empty(), "{:?}", project.warnings);
    for main in [
        "const { BrowserWindow } = require('electron'); new BrowserWindow();",
        "const electron = require('electron'); new electron.BrowserWindow();",
    ] {
        headless.write("index.js", main.as_bytes());
        assert_eq!(
            load(headless.root()).unwrap().warnings,
            vec!["no renderer HTML page was found".to_owned()]
        );
    }
}

#[test]
fn electron_vite_projects() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name": "ev", "main": "./out/main/index.js", "devDependencies": {"electron": "1", "electron-vite": "2"}}"#,
        ),
        (
            "electron.vite.config.ts",
            "import {resolve} from 'path'; export default { renderer: { resolve: { alias: { '@renderer': resolve('src/renderer/src') } } } }",
        ),
        (
            "tsconfig.json",
            r#"{"compilerOptions": {"paths": {"@renderer/*": ["src/renderer/src/*"]}}}"#,
        ),
        ("src/main/index.ts", "import { app } from 'electron';"),
        ("src/preload/index.ts", ""),
        ("src/renderer/index.html", "<html></html>"),
        ("src/renderer/settings.html", "<html></html>"),
        ("src/renderer/notes.txt", ""),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.layout, Layout::ElectronVite);
    assert_eq!(
        paths(std::slice::from_ref(&project.main), fixture.root()),
        owned(&[("src/main/index.ts", &["out/main/index.js"])])
    );
    assert_eq!(
        paths(&project.preloads, fixture.root()),
        owned(&[(
            "src/preload/index.ts",
            &["out/preload/index.js", "out/preload/index.mjs"]
        )])
    );
    assert_eq!(
        paths(&project.pages, fixture.root()),
        owned(&[
            ("src/renderer/index.html", &["out/renderer/index.html"]),
            (
                "src/renderer/settings.html",
                &["out/renderer/settings.html"]
            ),
        ])
    );
    let patterns: Vec<&str> = project
        .aliases
        .iter()
        .map(|alias| alias.pattern.as_str())
        .collect();
    assert_eq!(patterns, vec!["@renderer/*"]);
    assert_eq!(project.renderer_aliases[0].pattern, "@renderer");
    assert_eq!(project.renderer_aliases[1].pattern, "@renderer/*");
    let bare = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "out/main/index.js", "devDependencies": {"electron": "1"}}"#,
        ),
        ("electron.vite.config.mjs", "export default {}"),
        ("src/main/index.js", ""),
    ]);
    let project = load(bare.root()).unwrap();
    assert_eq!(project.layout, Layout::ElectronVite);
    assert!(project.preloads.is_empty());
}

#[test]
fn vite_plugin_electron_projects() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "dist-electron/main.js", "devDependencies": {"electron": "1", "vite-plugin-electron": "1", "react": "18"}}"#,
        ),
        (
            "vite.config.ts",
            "export default { resolve: { alias: { '@': path.resolve(__dirname, 'src') } } }",
        ),
        ("electron/main.ts", ""),
        ("electron/preload.ts", ""),
        ("index.html", "<html></html>"),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.layout, Layout::VitePluginElectron);
    assert_eq!(
        project.main.runtime_paths,
        vec!["dist-electron/main.js".to_owned()]
    );
    assert_eq!(project.preloads[0].runtime_paths.len(), 4);
    assert_eq!(
        paths(&project.pages, fixture.root()),
        owned(&[("index.html", &["dist/index.html"])])
    );
    assert_eq!(project.aliases.len(), 2);
}

#[test]
fn forge_vite_projects() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": ".vite/build/main.js", "devDependencies": {"electron": "1", "@electron-forge/plugin-vite": "7"}}"#,
        ),
        (
            "src/main.ts",
            "if (MAIN_WINDOW_VITE_DEV_SERVER_URL) { win.loadURL(MAIN_WINDOW_VITE_DEV_SERVER_URL); } \
             else { win.loadFile(path.join(__dirname, `../renderer/${MAIN_WINDOW_VITE_NAME}/index.html`)); }",
        ),
        ("src/preload.ts", ""),
        ("index.html", "<html></html>"),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.layout, Layout::ForgeVite);
    assert_eq!(
        project.defines,
        vec![
            (
                "MAIN_WINDOW_VITE_DEV_SERVER_URL".to_owned(),
                "undefined".to_owned()
            ),
            (
                "MAIN_WINDOW_VITE_NAME".to_owned(),
                "\"main_window\"".to_owned()
            ),
        ]
    );
    assert_eq!(
        project.preloads[0].runtime_paths,
        vec![".vite/build/preload.js".to_owned()]
    );
    assert_eq!(
        paths(&project.pages, fixture.root()),
        owned(&[("index.html", &[".vite/renderer/main_window/index.html"])])
    );
}

#[test]
fn built_output_is_used_when_sources_are_missing() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "dist-electron/main.js", "devDependencies": {"electron": "1", "vite-plugin-electron": "1"}}"#,
        ),
        ("dist-electron/main.js", ""),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.main.source, fixture.path("dist-electron/main.js"));
    assert!(project.preloads.is_empty());
}

#[test]
fn typescript_projects_compiled_by_tsc() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name": "ts-app", "main": "dist/src/main/main.js", "type": "module",
                "devDependencies": {"electron": "41"},
                "scripts": {"build": "tsc --project tsconfig.json && node scripts/copy-assets.mjs"},
                "build": {"productName": "TS App", "extraResources": [{"from": "sidecars", "to": "sidecars"}]}}"#,
        ),
        (
            "tsconfig.json",
            r#"{"compilerOptions": {"module": "NodeNext", "rootDir": ".", "outDir": "dist",},
                "include": ["src/**/*.ts", "src/**/*.cts"]}"#,
        ),
        (
            "src/main/main.ts",
            "import electron from 'electron';\nimport { createWindow } from './window.js';\nimport './preload-helpers.js';\n\
             electron.app.on('ready', createWindow);\n",
        ),
        (
            "src/main/window.ts",
            "import { join } from 'node:path';\nimport electron from 'electron';\nconst { app, BrowserWindow } = electron;\n\
             export async function createWindow(): Promise<void> {\n\
             const preloadPath = join(app.getAppPath(), 'dist', 'src', 'preload', 'preload.cjs');\n\
             const window = new BrowserWindow({ webPreferences: { preload: preloadPath } });\n\
             const second = new BrowserWindow({ webPreferences: { preload: join(__dirname, '../preload/second-preload.js') } });\n\
             const helper = { preload: join(__dirname, 'preload-helpers.js') };\n\
             await window.loadFile(join(app.getAppPath(), 'dist', 'src', 'renderer', 'index.html'));\n}\n",
        ),
        ("src/main/preload-helpers.ts", "export const helpers = 1;"),
        (
            "src/preload/preload.cts",
            "import electron = require('electron');",
        ),
        ("src/preload/second-preload.ts", ""),
        (
            "src/renderer/index.html",
            "<script type=module src=./app.js></script>",
        ),
        ("src/renderer/app.ts", ""),
        ("sidecars/server.exe", "exe"),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.layout, Layout::Plain);
    assert_eq!(project.metadata.product_name, "TS App");
    assert_eq!(project.output_directory, "release");
    assert_eq!(
        project.emits,
        vec![Emit {
            root_dir: String::new(),
            out_dir: "dist".to_owned()
        }]
    );
    assert_eq!(
        paths(std::slice::from_ref(&project.main), fixture.root()),
        owned(&[(
            "src/main/main.ts",
            &[
                "dist/src/main/main.js",
                "src/main/main.js",
                "src/main/main.ts"
            ]
        )])
    );
    assert_eq!(
        paths(&project.preloads, fixture.root()),
        owned(&[
            (
                "src/preload/preload.cts",
                &[
                    "dist/src/preload/preload.cjs",
                    "src/preload/preload.cjs",
                    "src/preload/preload.cts"
                ]
            ),
            (
                "src/preload/second-preload.ts",
                &[
                    "src/preload/second-preload.js",
                    "dist/src/preload/second-preload.js",
                    "src/preload/second-preload.ts"
                ]
            ),
        ])
    );
    assert_eq!(
        paths(&project.pages, fixture.root()),
        owned(&[(
            "src/renderer/index.html",
            &["dist/src/renderer/index.html", "src/renderer/index.html"]
        )])
    );
    assert_eq!(
        project.extra_files,
        vec![builder::ExtraFile {
            source: fixture.path("sidecars/server.exe"),
            destination: "resources/sidecars/server.exe".to_owned(),
        }]
    );
    assert!(project.warnings.is_empty());
    // A root given as `.` from the project directory finds the same files.
    let dotted = load(&fixture.root().join(".")).unwrap();
    assert_eq!(dotted.root, fixture.root());
    assert_eq!(dotted.extra_files, project.extra_files);
}

#[test]
fn compiled_entries_are_found_without_a_typescript_configuration() {
    let project = |main: &str, files: &[(&str, &str)]| {
        let manifest = format!(r#"{{"main": "{main}", "devDependencies": {{"electron": "30"}}}}"#);
        let mut all = vec![("package.json", manifest.as_str())];
        all.extend_from_slice(files);
        let fixture = Fixture::new(&all);
        load(fixture.root()).map(|project| {
            (
                project.emits.clone(),
                paths(std::slice::from_ref(&project.main), fixture.root()),
            )
        })
    };
    let emit = |root_dir: &str, out_dir: &str| Emit {
        root_dir: root_dir.to_owned(),
        out_dir: out_dir.to_owned(),
    };
    assert_eq!(
        project("dist/main.js", &[("src/main.ts", "")]).unwrap(),
        (
            vec![emit("src", "dist")],
            owned(&[(
                "src/main.ts",
                &["dist/main.js", "src/main.js", "src/main.ts"]
            )])
        )
    );
    assert_eq!(
        project("build/index.js", &[("index.ts", "")]).unwrap(),
        (
            vec![emit("", "build")],
            owned(&[("index.ts", &["build/index.js", "index.js", "index.ts"])])
        )
    );
    assert_eq!(
        project("dist/missing.js", &[]).unwrap_err(),
        "cannot find the main process entry \"dist/missing.js\""
    );
    assert!(project("other/main.js", &[("src/main.ts", "")]).is_err());
}

/// A project that fetches Electron itself (as VS Code does) names the
/// version its native modules are built against in `.npmrc`, and compiles
/// `src` into the `out` directory `main` names.
#[test]
fn npmrc_electron_targets_make_electron_projects() {
    let fixture = Fixture::new(&[
        ("package.json", r#"{"main": "./out/main.js"}"#),
        (
            ".npmrc",
            "# node-gyp\ndisturl=\"https://electronjs.org/headers\"\ntarget=\"43.8.3\"\n; comment\n runtime = \"electron\"\n",
        ),
        ("src/main.ts", ""),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.dependencies["electron"], "43.8.3");
    assert_eq!(project.main.source, fixture.root().join("src/main.ts"));
}

/// Resource paths relative to the compiled modules' root, as VS Code's
/// `FileAccess.asBrowserUri('vs/…/workbench.html')`, name the pages and
/// preload scripts `src` holds, loaded from `out`.
#[test]
fn resource_paths_resolve_against_the_output_root() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "./out/main.js", "devDependencies": {"electron": "43"}}"#,
        ),
        ("src/main.ts", "import './vs/platform/window.js';\n"),
        (
            "src/vs/platform/window.ts",
            "import { BrowserWindow } from 'electron';\n\
             const win = new BrowserWindow({ webPreferences: { preload: FileAccess.asFileUri('vs/sandbox/preload.js').fsPath } });\n\
             win.loadURL(FileAccess.asBrowserUri(`vs/code/workbench${built ? '' : '-dev'}.html`).toString(true));\n",
        ),
        ("src/vs/sandbox/preload.ts", ""),
        ("src/vs/code/workbench.html", "<html></html>"),
        ("src/vs/code/workbench-dev.html", "<html></html>"),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(
        paths(&project.pages, fixture.root()),
        owned(&[
            (
                "src/vs/code/workbench.html",
                &["out/vs/code/workbench.html", "src/vs/code/workbench.html"],
            ),
            (
                "src/vs/code/workbench-dev.html",
                &[
                    "out/vs/code/workbench-dev.html",
                    "src/vs/code/workbench-dev.html"
                ],
            ),
        ])
    );
    assert_eq!(
        paths(&project.preloads, fixture.root()),
        owned(&[(
            "src/vs/sandbox/preload.ts",
            &[
                "out/vs/sandbox/preload.js",
                "src/vs/sandbox/preload.js",
                "src/vs/sandbox/preload.ts"
            ],
        )])
    );
}

#[test]
fn invalid_projects_are_rejected() {
    let empty = Fixture::new(&[]);
    assert!(load(empty.root()).unwrap_err().starts_with("cannot read "));
    let broken = Fixture::new(&[("package.json", "{")]);
    assert!(load(broken.root()).unwrap_err().starts_with("invalid "));
    let web = Fixture::new(&[("package.json", r#"{"dependencies": {"react": "18"}}"#)]);
    assert!(
        load(web.root())
            .unwrap_err()
            .ends_with("is not an Electron project: package.json does not depend on electron")
    );
    // node-gyp's `.npmrc` runtime alone does not name an Electron version.
    let runtime_only = Fixture::new(&[
        ("package.json", r#"{"main": "main.js"}"#),
        (".npmrc", "runtime=electron\n"),
        ("main.js", ""),
    ]);
    assert!(load(runtime_only.root()).is_err());
    let node_target = Fixture::new(&[
        ("package.json", r#"{"main": "main.js"}"#),
        (".npmrc", "runtime=node\ntarget=22.0.0\n"),
        ("main.js", ""),
    ]);
    assert!(load(node_target.root()).is_err());
    let no_main = Fixture::new(&[(
        "package.json",
        r#"{"main": "app.js", "dependencies": {"electron": "1"}}"#,
    )]);
    assert_eq!(
        load(no_main.root()).unwrap_err(),
        "cannot find the main process entry \"app.js\""
    );
}

#[test]
fn helpers() {
    assert_eq!(relative(Path::new("/a"), Path::new("/b/c")), "/b/c");
    assert_eq!(file_name(Path::new("")), "");
    assert!(html_files(Path::new("Z:/definitely/missing")).is_empty());
}
#[test]
fn linked_project_roots_share_physical_sources_and_metadata() {
    let fixture = fixture::Fixture::new(&[
        (
            "application/package.json",
            r#"{"name":"linked","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        (
            "application/main.js",
            "const {BrowserWindow}=require('electron');new BrowserWindow().loadFile('index.html');",
        ),
        ("application/index.html", "<p>linked</p>"),
    ]);
    fixture.junction("linked application", "application");
    let project = load(&fixture.path("linked application")).unwrap();
    assert_eq!(project, load(&fixture.path("application")).unwrap());
    assert_eq!(project.root, fixture.path("application"));
    assert_eq!(project.main.source, fixture.path("application/main.js"));
    assert_eq!(
        project.pages[0].source,
        fixture.path("application/index.html")
    );
}

/// A forked module named relative to the compiled output's root is found
/// as a source, with the paths it is loaded by.
#[test]
fn forked_modules_are_entries() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "./out/main.js", "devDependencies": {"electron": "43"}}"#,
        ),
        (
            "src/main.ts",
            "import { utilityProcess } from 'electron'; const modulePath = FileAccess.asFileUri('bootstrap-fork.js').fsPath; utilityProcess.fork(modulePath);",
        ),
        ("src/bootstrap-fork.ts", ""),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(
        paths(&project.forks, fixture.root()),
        owned(&[(
            "src/bootstrap-fork.ts",
            &[
                "src/bootstrap-fork.js",
                "out/bootstrap-fork.js",
                "src/bootstrap-fork.ts",
            ],
        )])
    );
}

/// A module forked by a path relative to the compiled modules' root, from a
/// file in another folder, as VS Code's utility processes fork
/// `FileAccess.asFileUri('bootstrap-fork.js')`, is also found by its
/// source's path with the emitted extension, which a native build's module
/// URLs name.
#[test]
fn forked_modules_are_found_by_their_sources_emitted_names() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "./out/main.js", "devDependencies": {"electron": "43"}}"#,
        ),
        ("src/main.ts", "import './vs/process/start.js';"),
        (
            "src/vs/process/start.ts",
            "import { utilityProcess } from 'electron'; const modulePath = FileAccess.asFileUri('bootstrap-fork.js').fsPath; utilityProcess.fork(modulePath);",
        ),
        ("src/bootstrap-fork.ts", ""),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(
        paths(&project.forks, fixture.root()),
        owned(&[(
            "src/bootstrap-fork.ts",
            &[
                "out/bootstrap-fork.js",
                "src/bootstrap-fork.js",
                "src/bootstrap-fork.ts",
            ],
        )])
    );
    assert_eq!(emitted_in_place("src/a.mts").as_deref(), Some("src/a.mjs"));
    assert_eq!(emitted_in_place("src/a.tsx").as_deref(), Some("src/a.jsx"));
    assert_eq!(emitted_in_place("src/a.d.ts"), None);
    assert_eq!(emitted_in_place("src/a.js"), None);
}
