pub(crate) mod fixture;

use super::*;
use catalog::Support::{Partial, Supported, Unsupported};
use fixture::Fixture;

fn sample() -> Fixture {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name": "sample", "main": "main.js",
                "dependencies": {"react": "18", "react-dom": "18", "better-sqlite3": "9", "gyp-pkg": "1", "flagged": "1", "plain": "1", "broken": "1"},
                "devDependencies": {"electron": "30"}}"#,
        ),
        (
            "main.js",
            "const { app, BrowserWindow } = require('electron');\nconst fs = require('fs');\n\
             app.whenReady().then(() => new BrowserWindow().loadFile('index.html'));\n",
        ),
        (
            "index.html",
            "<html><head><link rel=\"stylesheet\" href=\"./src/binary.css\"></head><body><div id=\"root\"></div><script src=\"./src/main.tsx\"></script></body></html>",
        ),
        (
            "src/main.tsx",
            "import { createRoot } from 'react-dom/client';\n\
             const App = () => <div className=\"toolbar\"><button>Save</button></div>;\n\
             createRoot(document.getElementById('root')).render(<App />);\n\
             const gl = canvas.getContext('webgl');\n\
             import './app.css';\nimport { ctx } from './chart';\nimport './long.js';\n",
        ),
        ("src/unused.ts", "canvas.getContext('webgl');"),
        (
            "src/chart.ts",
            "export const ctx = (canvas: HTMLCanvasElement) => canvas.getContext('2d');\n",
        ),
        ("src/types.d.ts", "declare const x: number;"),
        (
            "src/app.css",
            ".toolbar { display: flex; transition: all 1s; }\n",
        ),
        (
            "src/long.js",
            &format!(
                "const value = '{}'; fetch(value);\nfetch(a); fetch(b); fetch(c); fetch(d); fetch(e);\n",
                "x".repeat(120)
            ),
        ),
        ("node_modules/gyp-pkg/binding.gyp", "{}"),
        ("node_modules/flagged/package.json", "{\"gypfile\": true}"),
        ("node_modules/plain/package.json", "{}"),
        ("node_modules/broken/package.json", "{"),
        ("node_modules/react/index.js", "canvas.getContext('webgl');"),
        ("dist/bundle.js", "canvas.getContext('webgl');"),
        (".cache/x.js", "canvas.getContext('webgl');"),
        ("README.md", "# readme"),
    ]);
    fixture.write("src/binary.css", &[0xff, 0xfe]);
    fixture
}

#[test]
fn shipped_files_resolve_main_preload_and_renderer_aliases_independently() {
    let source = "import 'phase'; import path from 'path'; path.join('a','b');";
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"out/main/index.js","devDependencies":{"electron":"42"}}"#,
        ),
        (
            "electron.vite.config.ts",
            r#"import {resolve} from 'path'; export default {
main:{resolve:{alias:{phase:resolve(__dirname,'main.ts')}}},
preload:{resolve:{alias:{phase:resolve(__dirname,'preload.ts')}}},
renderer:{resolve:{alias:{phase:resolve(__dirname,'renderer.ts'),path:'./browser-path.ts'}}}};"#,
        ),
        ("src/main/index.ts", source),
        ("src/preload/index.ts", source),
        (
            "src/renderer/index.html",
            "<script src='./main.ts'></script>",
        ),
        ("src/renderer/main.ts", source),
        ("main.ts", ""),
        ("preload.ts", ""),
        ("renderer.ts", ""),
        (
            "src/renderer/browser-path.ts",
            "canvas.getContext('webgl');",
        ),
    ]);
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let files = shipped_files(&project);
    for path in [
        "main.ts",
        "preload.ts",
        "renderer.ts",
        "src/renderer/browser-path.ts",
    ] {
        assert!(files.contains(&fixture.path(path)), "{files:?}");
    }
    let report = analyze(&project);
    let feature = report
        .features
        .iter()
        .find(|used| used.feature.id == "web:webgl")
        .unwrap();
    assert_eq!(feature.occurrences[0].file, "src/renderer/browser-path.ts");
    assert_eq!(feature.feature.support, Unsupported);
}

#[test]
fn shipped_files_follow_the_application() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "main.js", "devDependencies": {"electron": "30"}}"#,
        ),
        (
            "main.js",
            "const path = require('path');\nrequire('./lib/util.js');\n\
             const win = new BrowserWindow({ webPreferences: { preload: path.join(__dirname, 'preload.js') } });\n\
             win.loadFile('index.html');\n",
        ),
        ("lib/util.js", ""),
        ("preload.js", "require('./preload-lib.js');"),
        ("preload-lib.js", ""),
        (
            "index.html",
            "<link rel=stylesheet href=\"/styles/site.css\"><link rel=STYLESHEET href=\"https://cdn.example/x.css\">\
             <link rel=icon href=icon.png><link href=nothing.css><script src=\"https://cdn.example/x.js\"></script>\
             <script src=\"./app.js?v=1\"></script><script src=\"./missing.js\"></script><script>inline()</script>",
        ),
        ("styles/site.css", ""),
        ("app.js", "import './view.js';\nimport './view.css';\n"),
        ("view.js", ""),
        ("view.css", ""),
        ("nothing.css", ""),
        ("unused.js", ""),
    ]);
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let files: Vec<String> = shipped_files(&project)
        .iter()
        .map(|file| {
            file.strip_prefix(fixture.root())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(
        files,
        vec![
            "app.js",
            "index.html",
            "lib/util.js",
            "main.js",
            "preload-lib.js",
            "preload.js",
            "styles/site.css",
            "view.css",
            "view.js"
        ]
    );
    assert_eq!(
        page_url(
            Path::new("C:/app"),
            Path::new("C:/app/pages/a.html"),
            "x.js#y"
        ),
        Path::new("C:/app/pages/x.js")
    );
}

#[test]
fn features_used_anywhere_in_a_less_supported_way_are_less_supported() {
    let mut collector = Collector {
        features: Vec::new(),
    };
    let occurrence = Occurrence {
        file: "a.js".to_owned(),
        line: 1,
        column: 1,
        snippet: String::new(),
    };
    collector.add(catalog::node_export("net", "isIP"), occurrence.clone());
    collector.add(catalog::node("net"), occurrence.clone());
    collector.add(catalog::node_export("net", "isIP"), occurrence);
    assert_eq!(collector.features.len(), 1);
    assert_eq!(collector.features[0].feature.support, Partial);
    assert_eq!(collector.features[0].occurrences.len(), 3);
}

#[test]
fn electron_vite_scans_configured_preloads_and_root_relative_renderer_modules() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"output/main.js","devDependencies":{"electron":"44","electron-vite":"5"}}"#,
        ),
        (
            "electron.vite.config.ts",
            "export default {main:{build:{lib:{entry:'process/start.ts'}}},preload:{build:{lib:{entry:'process/bridge.ts'}}},renderer:{root:'view'}}",
        ),
        ("process/start.ts", "import {app} from 'electron';"),
        (
            "process/bridge.ts",
            "require('electron').contextBridge.exposeInMainWorld('api',{});",
        ),
        (
            "view/index.html",
            "<script type=module src=/entry.tsx></script><link rel=stylesheet href=/style.css>",
        ),
        (
            "view/entry.tsx",
            "document.body.innerHTML='<main>Source</main>';",
        ),
        ("view/style.css", "body {display:flex}"),
        ("entry.tsx", "canvas.getContext('webgl');"),
    ]);
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let files = shipped_files(&project);
    assert!(files.contains(&fixture.path("view/entry.tsx")));
    assert!(files.contains(&fixture.path("view/style.css")));
    assert!(files.contains(&fixture.path("process/bridge.ts")));
    assert!(!files.contains(&fixture.path("entry.tsx")));
    assert!(
        !analyze(&project)
            .features
            .iter()
            .any(|feature| feature.feature.id == "web:webgl")
    );
}

#[test]
fn projects_are_scored_and_reported() {
    let fixture = sample();
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let report = analyze(&project);
    assert_eq!(report.files, 7);
    assert!(report.has_unsupported());
    let names: Vec<(&str, Support)> = report
        .features
        .iter()
        .map(|used| (used.feature.name.as_str(), used.feature.support))
        .collect();
    assert_eq!(
        names,
        vec![
            ("HTML elements", Supported),
            ("Electron app", Supported),
            ("Electron BrowserWindow", Supported),
            ("Node fs", Supported),
            ("Flexbox", Supported),
            ("CSS transitions", Supported),
            ("Canvas", Supported),
            ("Form controls", Supported),
            ("fetch", Partial),
            ("React DOM", Partial),
            ("React", Partial),
            ("Native module better-sqlite3", Unsupported),
            ("Native module flagged", Unsupported),
            ("Native module gyp-pkg", Unsupported),
            ("WebGL", Unsupported),
        ]
    );
    assert_eq!(report.score, 63);
    let webgl = report
        .features
        .iter()
        .find(|used| used.feature.id == "web:webgl")
        .unwrap();
    assert_eq!(
        webgl.occurrences,
        vec![Occurrence {
            file: "src/main.tsx".to_owned(),
            line: 4,
            column: 12,
            snippet: "const gl = canvas.getContext('webgl');".to_owned(),
        }]
    );
    let text = report.to_text();
    assert!(
        text.starts_with("Project compatibility: 63%\n\nHTML elements                 Supported\n")
    );
    assert!(text.contains("\nWebGL (Unsupported): requires a GPU browser stack\n  src/main.tsx:4\n      const gl = canvas.getContext('webgl');\n"));
    assert!(text.contains("Native module flagged (Unsupported): native Node.js addons cannot be loaded by the native runtime\n  package.json\n"));
    assert!(text.contains("  … and 1 more\n"));
    assert!(text.contains("…\n"));
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(json["score"], 63);
}

/// TypeScript outside `.tsx` is parsed without JSX, so `<T>value` type
/// assertions parse and the file's features are reported; `.tsx` and `.js`
/// files keep JSX.
#[test]
fn typescript_type_assertions_are_not_jsx() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "main.js", "devDependencies": {"electron": "43"}}"#,
        ),
        (
            "main.js",
            "import './fork.ts';\nimport './view.tsx';\nimport './plain.js';\n",
        ),
        (
            "fork.ts",
            "import { utilityProcess } from 'electron';\nconst child = <Child>utilityProcess.fork('child.js');\n",
        ),
        (
            "view.tsx",
            "import { MessageChannelMain } from 'electron';\nexport const view = <div>{String(new MessageChannelMain())}</div>;\n",
        ),
        (
            "plain.js",
            "import { WebContentsView } from 'electron';\nexport const view = <b>{new WebContentsView()}</b>;\n",
        ),
    ]);
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let report = analyze(&project);
    let names: Vec<&str> = report
        .features
        .iter()
        .map(|used| used.feature.name.as_str())
        .collect();
    for name in [
        "Electron utilityProcess",
        "Electron MessageChannelMain",
        "Electron WebContentsView",
    ] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
}

#[test]
fn empty_projects_score_full_marks() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "main.js", "devDependencies": {"electron": "30"}}"#,
        ),
        ("main.js", "console.log(1);"),
    ]);
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let report = analyze(&project);
    assert_eq!(report.score, 100);
    assert!(!report.has_unsupported());
    assert_eq!(report.to_text(), "Project compatibility: 100%\n");
}

#[test]
fn unused_framework_tooling_is_ignored_but_loaded_preprocessors_are_rejected() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"main.ts","dependencies":{"react":"19"},"devDependencies":{"electron":"44","sass":"1","less":"4","stylus":"1","@angular/core":"20"}}"#,
        ),
        (
            "main.ts",
            "import {MenuItemConstructorOptions, IpcRendererEvent} from 'electron'; let menu: MenuItemConstructorOptions; let event: IpcRendererEvent;",
        ),
    ]);
    let project = nativeboat_project::load(fixture.root()).unwrap();
    assert_eq!(analyze(&project).score, 100);
    assert!(analyze(&project).features.is_empty());
    fixture.write("main.ts", b"require('electron').app.whenReady().then(()=>new (require('electron').BrowserWindow)().loadFile('index.html')); import './theme.scss';");
    for (extension, package) in [
        ("scss", "sass"),
        ("sass", "sass"),
        ("less", "less"),
        ("styl", "stylus"),
        ("stylus", "stylus"),
    ] {
        fixture.write(
            "index.html",
            format!("<link rel=stylesheet href=theme.{extension}>").as_bytes(),
        );
        fixture.write(
            &format!("theme.{extension}"),
            b"$color: red; body {color:$color}",
        );
        let project = nativeboat_project::load(fixture.root()).unwrap();
        let report = analyze(&project);
        let preprocessor = report
            .features
            .iter()
            .find(|used| used.feature.id == format!("framework:{package}"))
            .unwrap();
        assert_eq!(preprocessor.feature.support, Unsupported);
        assert!(
            preprocessor
                .occurrences
                .iter()
                .any(|occurrence| occurrence.file == format!("theme.{extension}")
                    && occurrence.line == 1)
        );
        assert!(report.has_unsupported());
    }
}

#[test]
fn snippets_and_lines() {
    assert_eq!(line_starts("a\nb\n"), vec![0, 2, 4]);
    assert_eq!(snippet("  a  \nb", 1), "a");
    assert_eq!(snippet("a", 5), "");
    assert_eq!(snippet("a", 0), "");
}

#[test]
fn experimental_decorators_are_supported() {
    let files = |config: &'static str| {
        Fixture::new(&[
            (
                "package.json",
                r#"{"main": "main.ts", "devDependencies": {"electron": "30"}}"#,
            ),
            (
                "main.ts",
                "function log(target: any) {}\n@log class Service {}\n",
            ),
            ("tsconfig.json", config),
        ])
    };
    let decorators = |fixture: &Fixture| {
        let project = nativeboat_project::load(fixture.root()).unwrap();
        analyze(&project)
            .features
            .into_iter()
            .find(|used| used.feature.id == "language:decorators")
            .unwrap()
            .feature
            .support
    };
    let legacy = files(r#"{"compilerOptions": {"experimentalDecorators": true}}"#);
    assert_eq!(decorators(&legacy), Supported);
    let standard = files(r#"{"compilerOptions": {}}"#);
    assert_eq!(decorators(&standard), Unsupported);
}

#[test]
fn decorator_metadata_keeps_implicit_runtime_imports_visible() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"experimentalDecorators":true,"emitDecoratorMetadata":true}}"#,
        ),
        (
            "main.ts",
            "import {BrowserWindow, type IpcRendererEvent} from 'electron'; function decorated(target: any) {} @decorated class Service { constructor(window: BrowserWindow, event: IpcRendererEvent) {} }",
        ),
    ]);
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let report = analyze(&project);
    assert!(
        report
            .features
            .iter()
            .any(|used| used.feature.id == "electron:BrowserWindow")
    );
    assert!(
        !report
            .features
            .iter()
            .any(|used| used.feature.id == "electron:IpcRendererEvent")
    );
    let options = nativeboat_bundler::TranspileOptions {
        legacy_decorators: true,
        decorator_metadata: true,
        ..Default::default()
    };
    let emitted = nativeboat_bundler::transpile::transpile(
        &std::fs::read_to_string(fixture.path("main.ts")).unwrap(),
        Path::new("main.ts"),
        &options,
    )
    .unwrap();
    assert!(emitted.contains("BrowserWindow"));
}

#[test]
fn gradient_reports_keep_the_least_supported_layer_across_files() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"main.js","devDependencies":{"electron":"30"}}"#,
        ),
        (
            "main.js",
            "require('electron').app.whenReady().then(() => new (require('electron').BrowserWindow)().loadFile('index.html'));",
        ),
        (
            "index.html",
            "<link rel=\"stylesheet\" href=\"app.css\"><style>.card{background:linear-gradient(180deg,#fffaf0,#fff4df)}</style>",
        ),
        (
            "app.css",
            ".other{background-image:radial-gradient(red,blue)}",
        ),
    ]);
    let support = || {
        let project = nativeboat_project::load(fixture.root()).unwrap();
        analyze(&project)
            .features
            .into_iter()
            .find(|used| used.feature.id == "css:gradient")
            .unwrap()
    };
    let mixed = support();
    assert_eq!(mixed.feature.support, Partial);
    assert_eq!(mixed.occurrences.len(), 2);
    fixture.write(
        "app.css",
        b".other{background-image:linear-gradient(to right,red,blue)}",
    );
    assert_eq!(support().feature.support, Supported);
}

#[test]
fn vue_components_are_analyzed_block_by_block() {
    let fixture = Fixture::new(&[
        (
            "app/package.json",
            r#"{"main": "main.js", "dependencies": {"vue": "3"}, "devDependencies": {"electron": "30"}}"#,
        ),
        (
            "app/main.js",
            "const { app, BrowserWindow } = require('electron');\n\
             app.whenReady().then(() => new BrowserWindow().loadFile('index.html'));\n",
        ),
        (
            "app/index.html",
            "<div id=\"app\"></div><script type=\"module\" src=\"./main.ts\"></script>",
        ),
        (
            "app/main.ts",
            "import { createApp } from 'vue';\nimport App from './App.vue';\ncreateApp(App).mount('#app');\n",
        ),
        (
            "app/App.vue",
            "<template>\n  <video src=\"a.mp4\"></video>\n</template>\n\n\
             <script setup lang=\"ts\">\nimport '../shared/theme.css'\n\
             const ctx = (canvas as HTMLCanvasElement).getContext('2d')\n</script>\n\n\
             <style scoped>\n.a { display: grid }\n</style>\n",
        ),
        ("shared/theme.css", ".b { float: left }\n"),
    ]);
    let project = nativeboat_project::load(&fixture.path("app")).unwrap();
    let report = analyze(&project);
    let found = |id: &str| {
        let used = report
            .features
            .iter()
            .find(|used| used.feature.id == id)
            .unwrap_or_else(|| panic!("{id} not found"));
        let first = &used.occurrences[0];
        (first.file.clone(), first.line, first.column)
    };
    assert_eq!(found("framework:vue").0, "main.ts");
    assert_eq!(found("web:canvas-2d"), ("App.vue".to_owned(), 7, 13));
    assert_eq!(found("css:grid"), ("App.vue".to_owned(), 11, 6));
    assert_eq!(found("css:float").0, "../shared/theme.css");
    assert!(
        report
            .features
            .iter()
            .any(|used| used.feature.id.starts_with("html:")
                && used.occurrences[0].file == "App.vue"
                && used.occurrences[0].line == 2)
    );
}

#[test]
fn selectors_vue_rewrites_in_scoped_styles_are_not_judged() {
    let fixture = Fixture::new(&[
        (
            "app/package.json",
            r#"{"main": "main.js", "dependencies": {"vue": "3"}, "devDependencies": {"electron": "30"}}"#,
        ),
        (
            "app/main.js",
            "const { app, BrowserWindow } = require('electron');\n\
             app.whenReady().then(() => new BrowserWindow().loadFile('index.html'));\n",
        ),
        (
            "app/index.html",
            "<div id=\"app\"></div><script type=\"module\" src=\"./main.ts\"></script>",
        ),
        (
            "app/main.ts",
            "import { createApp } from 'vue';\nimport App from './App.vue';\nimport Plain from './Plain.vue';\n\
             createApp(App).component('plain', Plain).mount('#app');\n",
        ),
        (
            "app/App.vue",
            "<template><div class=\"a\"></div></template>\n\
             <style scoped>\n.a :deep(.b), .c :v-slotted(.d), :global(.e) { color: red }\n</style>\n",
        ),
        (
            "app/Plain.vue",
            "<template><div class=\"x\"></div></template>\n\
             <style>\n.x :deep(.y) { color: red }\n</style>\n",
        ),
    ]);
    let project = nativeboat_project::load(&fixture.path("app")).unwrap();
    let report = analyze(&project);
    let files = |id: &str| -> Vec<String> {
        report
            .features
            .iter()
            .filter(|used| used.feature.id == id)
            .flat_map(|used| used.occurrences.iter().map(|at| at.file.clone()))
            .collect()
    };
    // Unscoped, Vue leaves `:deep()` alone and it never matches.
    assert_eq!(files("css:selector:deep"), ["Plain.vue"]);
    assert!(files("css:selector:v-slotted").is_empty());
    assert!(files("css:selector:global").is_empty());
}
