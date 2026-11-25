use super::*;
use crate::tests::fixture::Fixture;

#[test]
fn main_sources_are_followed_and_scanned() {
    let fixture = Fixture::new(&[
        (
            "main.ts",
            "import { app, BrowserWindow } from 'electron';\nimport { createWindow } from './window';\n\
             const lazy = import('./lazy');\nconst helper = require('./helper');\nconst other = import(name);\n\
             require(variable);\nimport dep from 'dep';\nimport data from './data.json';\napp.whenReady().then(createWindow);\n",
        ),
        (
            "window.ts",
            "import path from 'path';\nimport './window';\nexport function createWindow() {\n\
             const win = new BrowserWindow({ webPreferences: { preload: path.join(__dirname, 'preload.js') } });\n\
             win.loadFile('index.html');\nwin.loadFile(`settings.html`);\nwin.loadURL(`${base}/page.html`);\n\
             win.loadURL(MAIN_WINDOW_VITE_DEV_SERVER_URL);\nconst name = MAIN_WINDOW_VITE_NAME;\nconst again = MAIN_WINDOW_VITE_NAME;\n\
             const opts = { preload: `preload2.js`, other: 'x', [key]: 1, preload2: variable };\nconst unknown = { preload: variable };\n}\n",
        ),
        (
            "lazy.js",
            "module.exports = { preload: 'lazy-preload.js' };",
        ),
        ("helper.js", "module.exports = 'help.HTML';"),
        ("data.json", "{}"),
        ("node_modules/dep/index.js", "module.exports = 'dep.html';"),
    ]);
    let resolver = Resolver {
        root: fixture.root().to_path_buf(),
        target: nativeboat_bundler::Target::Main,
        aliases: Vec::new(),
    };
    let scan = scan_main(&fixture.path("main.ts"), &resolver);
    let files: Vec<String> = scan
        .files
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, vec!["main.ts", "helper.js", "lazy.js", "window.ts"]);
    let literals = |references: &[Reference]| -> Vec<String> {
        references
            .iter()
            .map(|reference| reference.literal.clone())
            .collect()
    };
    assert_eq!(
        literals(&scan.pages),
        vec!["help.HTML", "index.html", "settings.html", "page.html"]
    );
    assert_eq!(
        literals(&scan.preloads),
        vec!["lazy-preload.js", "preload.js", "preload2.js"]
    );
    assert_eq!(scan.pages[1].directory, fixture.root());
    assert_eq!(
        scan.forge_globals,
        vec![
            "MAIN_WINDOW_VITE_DEV_SERVER_URL".to_owned(),
            "MAIN_WINDOW_VITE_NAME".to_owned()
        ]
    );
}

#[test]
fn built_paths_name_pages_and_preload_scripts() {
    let fixture = Fixture::new(&[
        (
            "main.ts",
            "import { join, resolve } from 'node:path';\nimport './styles.css';\nimport 'dep/theme.css';\nimport './styles.css';\n\
             const preloadPath = join(app.getAppPath(), 'dist', 'src', 'preload', 'preload.cjs');\n\
             const options = { webPreferences: { preload: preloadPath } };\nconst other = { preload: unknownVariable };\n\
             win.loadFile(join(app.getAppPath(), 'dist', 'src', 'renderer', 'index.html'));\n\
             win.loadFile(path.resolve(__dirname, '../renderer/about.html'));\n\
             win.loadURL(`file://${__dirname}/../renderer/help.html`);\nwin.loadURL(`${base}/`);\n\
             const bridge = require.resolve('./bridge-preload.js');\nconst worker = new URL('./worker-preload.mjs', import.meta.url);\n\
             const broken = join(__dirname, 'pages', name, 'x.html');\nconst empty = join();\nconst computed = obj[key]('a.html');\n\
             const named = { preload: join(__dirname, 'named.js') };\n",
        ),
        ("styles.css", ""),
        ("node_modules/dep/theme.css", ""),
    ]);
    let resolver = Resolver {
        root: fixture.root().to_path_buf(),
        target: nativeboat_bundler::Target::Main,
        aliases: Vec::new(),
    };
    let scan = scan_main(&fixture.path("main.ts"), &resolver);
    assert_eq!(
        scan.files,
        vec![fixture.path("main.ts"), fixture.path("styles.css")]
    );
    let literals = |references: &[Reference]| -> Vec<String> {
        references
            .iter()
            .map(|reference| reference.literal.clone())
            .collect()
    };
    assert_eq!(
        literals(&scan.pages),
        vec![
            "dist/src/renderer/index.html",
            "index.html",
            "../renderer/about.html",
            "../renderer/help.html",
            "x.html",
            "a.html"
        ]
    );
    assert_eq!(
        literals(&scan.preloads),
        vec![
            "dist/src/preload/preload.cjs",
            "preload.cjs",
            "./bridge-preload.js",
            "./worker-preload.mjs",
            "named.js"
        ]
    );
}

#[test]
fn missing_entries_scan_nothing() {
    let fixture = Fixture::new(&[]);
    let resolver = Resolver {
        root: fixture.root().to_path_buf(),
        target: nativeboat_bundler::Target::Main,
        aliases: Vec::new(),
    };
    assert_eq!(
        scan_main(&fixture.path("missing.js"), &resolver),
        Scan::default()
    );
    assert!(is_script(Path::new("a.mts")));
    assert!(!is_script(Path::new("a.json")));
    assert!(!is_script(Path::new("noext")));
}

#[test]
fn shipped_files_include_components_and_preprocessed_stylesheets() {
    let fixture = Fixture::new(&[
        ("main.ts", "import App from './App.vue';\n"),
        (
            "App.vue",
            "<template><Child /></template>\n<script setup lang=\"ts\">\nimport Child from './Child.vue'\n\
             import './theme.scss'\nimport dep from 'dep'\nconst n: number = 1\n</script>\n",
        ),
        (
            "Child.vue",
            "<script>\nimport util from './util'\n</script>\n",
        ),
        ("util.ts", "export default 1;\n"),
        ("theme.scss", "$a: 1;\n"),
        ("node_modules/dep/index.js", "module.exports = 1;"),
    ]);
    let resolver = Resolver {
        root: fixture.root().to_path_buf(),
        target: nativeboat_bundler::Target::Renderer,
        aliases: Vec::new(),
    };
    let names = |scan: Scan| {
        let mut names: Vec<String> = scan
            .files
            .iter()
            .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(
        names(scan_shipped(&fixture.path("main.ts"), &resolver)),
        vec!["App.vue", "Child.vue", "main.ts", "theme.scss", "util.ts"]
    );
    assert_eq!(
        names(scan_main(&fixture.path("main.ts"), &resolver)),
        vec!["main.ts"]
    );
}

/// Template literals whose substitutions are literals or choices between
/// them name every page they can spell, as VS Code names its window page
/// `` `workbench${isBuilt ? '' : '-dev'}.html` ``.
#[test]
fn template_choices_name_every_page() {
    let fixture = Fixture::new(&[(
        "main.ts",
        "const built = process.env.BUILT;
         load(`vs/workbench${built ? '' : '-dev'}.html`);
         load(`${built ? 'a' : (other ? 'b' : `c`)}/${'x'}.html`);
         load(`${built ? 'a' : 'a'}/same.html`);
         load(`${dir}/${built ? 'p' : 'q'}.html`);
         load(`${built ? name : 'n'}/mixed.html`);
",
    )]);
    let resolver = Resolver {
        root: fixture.root().to_path_buf(),
        target: nativeboat_bundler::Target::Main,
        aliases: Vec::new(),
    };
    let scan = scan_main(&fixture.path("main.ts"), &resolver);
    let pages: Vec<&str> = scan
        .pages
        .iter()
        .map(|reference| reference.literal.as_str())
        .collect();
    assert_eq!(
        pages,
        vec![
            "vs/workbench.html",
            "vs/workbench-dev.html",
            "a/x.html",
            "b/x.html",
            "c/x.html",
            "a/same.html",
            // Other substitutions still give the text after the last one.
            ".html",
            "mixed.html",
        ]
    );
}

#[test]
fn template_choices_are_bounded() {
    let choice = "${f ? 'a' : 'b'}";
    let template = format!("`{}.html`", choice.repeat(5));
    let allocator = Allocator::default();
    let source = format!("load({template});");
    let parsed = Parser::new(&allocator, &source, SourceType::mjs()).parse();
    let oxc::ast::ast::Statement::ExpressionStatement(statement) = &parsed.program.body[0] else {
        panic!("an expression statement");
    };
    let Expression::CallExpression(call) = &statement.expression else {
        panic!("a call");
    };
    let Some(Expression::TemplateLiteral(literal)) = call.arguments[0].as_expression() else {
        panic!("a template literal");
    };
    // 32 spellings are more than a page reference is worth expanding.
    assert!(template_choices(literal).is_empty());
    assert_eq!(template_path(literal).as_deref(), Some(".html"));
}

/// `import(variable)` follows the paths the variable is given, as VS Code's
/// window script imports the workbench through `workbenchUrl`.
#[test]
fn imported_variables_follow_their_paths() {
    let fixture = Fixture::new(&[
        (
            "page/boot.ts",
            "let url: string;\nif (dev) { url = '../workbench/main.js'; } else { url = new URL(`vs/workbench/main.js`, base).href; }\nurl += '';\nconst result = await import(url);\nlet other = './other.js';\nimport(other);\nimport(unknown);\n",
        ),
        ("workbench/main.ts", "export const main = 1;"),
        ("page/other.ts", "export {};"),
    ]);
    let resolver = Resolver {
        root: fixture.root().to_path_buf(),
        target: nativeboat_bundler::Target::Renderer,
        aliases: Vec::new(),
    };
    let scan = scan_main(&fixture.path("page/boot.ts"), &resolver);
    let files: Vec<String> = scan
        .files
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, vec!["boot.ts", "other.ts", "main.ts"]);
}

/// Pages loaded through an application's own scheme, as
/// `win.loadURL('app://./index.html')` after `createProtocol('app')`, name
/// the application's files; web and file URLs keep their text.
#[test]
fn custom_scheme_urls_name_application_pages() {
    assert_eq!(
        custom_url_path("app://./index.html").as_deref(),
        Some("index.html")
    );
    assert_eq!(
        custom_url_path("my-app://bundle/pages/settings.html?tab=1#x").as_deref(),
        Some("pages/settings.html")
    );
    assert_eq!(custom_url_path("https://example.com/index.html"), None);
    assert_eq!(custom_url_path("file:///C:/app/index.html"), None);
    assert_eq!(custom_url_path("app://host"), None);
    assert_eq!(custom_url_path("index.html"), None);
    assert_eq!(custom_url_path("c://x/index.html"), None);
    let fixture = Fixture::new(&[(
        "main.js",
        "win.loadURL('app://./index.html'); win.loadURL('https://example.com/remote.html');",
    )]);
    let resolver = Resolver {
        root: fixture.root().to_path_buf(),
        target: nativeboat_bundler::Target::Main,
        aliases: Vec::new(),
    };
    let scan = scan_main(&fixture.path("main.js"), &resolver);
    let pages: Vec<&str> = scan
        .pages
        .iter()
        .map(|reference| reference.literal.as_str())
        .collect();
    assert_eq!(pages, vec!["index.html", "https://example.com/remote.html"]);
}

/// Modules started with `fork()` (Electron's `utilityProcess.fork`, Node's
/// `child_process.fork`) are found from their path, a variable given one,
/// or a URI's `fsPath`, as VS Code names `bootstrap-fork.js`.
#[test]
fn forked_modules_are_found() {
    let fixture = Fixture::new(&[(
        "main.ts",
        "import { utilityProcess } from 'electron'; import cp from 'child_process'; \
         const modulePath = FileAccess.asFileUri('bootstrap-fork.js').fsPath; \
         utilityProcess.fork(modulePath, []); \
         cp.fork(path.join(__dirname, 'worker.js')); \
         fork(`${base}/host.js`); \
         other.fork(unknown);",
    )]);
    let resolver = Resolver {
        root: fixture.root().to_path_buf(),
        target: nativeboat_bundler::Target::Main,
        aliases: Vec::new(),
    };
    let scan = scan_main(&fixture.path("main.ts"), &resolver);
    let forks: Vec<&str> = scan
        .forks
        .iter()
        .map(|reference| reference.literal.as_str())
        .collect();
    assert_eq!(forks, vec!["worker.js", "host.js", "bootstrap-fork.js"]);
}
