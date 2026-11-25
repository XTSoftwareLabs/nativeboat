use super::*;
use crate::tests::fixture::Fixture;
use std::process::Command;

const BUNDLE: &str = "node_modules/@vue/compiler-sfc/dist/compiler-sfc.esm-browser.js";

/// A stand-in compiler whose components are JSON descriptions of their
/// blocks and of how the compiler should behave.
const STUB: &str = r#"import fs from 'node:fs';
export function parse(source, options) {
  const spec = JSON.parse(source);
  if (spec.exit !== undefined) process.exit(spec.exit);
  if (spec.write !== undefined) {
    fs.writeFileSync(process.argv[process.argv.length - 1], spec.write);
    process.exit(0);
  }
  if (spec.throw) throw new Error(spec.throw);
  return {
    errors: spec.parseErrors || [],
    descriptor: {
      filename: options.filename,
      script: spec.script ? { content: spec.script } : null,
      scriptSetup: spec.setup ? { content: spec.setup, lang: spec.lang } : null,
      template: spec.template === undefined ? null : { content: spec.template },
      styles: spec.styles || [],
      slotted: false,
      cssVars: [],
    },
  };
}
export function compileScript(descriptor, options) {
  const script = descriptor.scriptSetup || descriptor.script;
  return { content: script.content + (options.inlineTemplate ? ' /* inline */' : ''), bindings: {} };
}
export function compileTemplate(options) {
  const errors = options.source === 'bad' ? [{ message: 'bad template' }] : [];
  return { code: '\nexport function render() { return ' + JSON.stringify(options.source) + '; }', errors };
}
export function compileStyle(options) {
  const code = options.scoped ? options.source.replace('{', '[' + options.id + ']{') : options.source;
  return { code, errors: options.source.includes('bad') ? ['bad style'] : [] };
}
"#;

fn settings(fixture: &Fixture) -> Args {
    Args {
        project: fixture.root().to_path_buf(),
        out: Some("native-output".into()),
        rustc: Some("rustc".into()),
        work_dir: None,
        sign: None,
        installer: None,
        config: None,
    }
}

/// An application with one renderer page.
fn app() -> Fixture {
    Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-vue-tool","main":"main.cjs","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.cjs",
            "const { BrowserWindow } = require('electron'); new BrowserWindow({ show: false }).loadFile('src/index.html');",
        ),
        (
            "src/index.html",
            "<!doctype html><script type=module src='./renderer.js'></script>",
        ),
        ("src/renderer.js", "import App from './App.vue'; void App;"),
    ])
}

/// A path whose separators `cmd` cannot mistake for switches.
fn windows_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

fn key(bundle: &Path) -> String {
    let contents = std::fs::read(bundle).unwrap();
    let source = driver(bundle);
    let key = crate::native::fingerprint(&[contents.as_slice(), source.as_bytes()]);
    format!("vue-sfc-{key:016x}")
}

#[test]
fn locates_the_compiler_installed_directly_or_beside_vue() {
    let fixture = Fixture::new(&[]);
    assert_eq!(locate(&fixture.path("app/src")), None);
    // pnpm links `node_modules/vue` into its store, beside `@vue/*`.
    let store = "store/node_modules/vue@3/node_modules";
    fixture.write(&format!("{store}/vue/package.json"), b"{}");
    std::fs::create_dir_all(fixture.path("app/node_modules")).unwrap();
    let linked = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(windows_path(&fixture.path("app/node_modules/vue")))
        .arg(windows_path(&fixture.path(&format!("{store}/vue"))))
        .output()
        .unwrap();
    assert!(linked.status.success(), "{linked:?}");
    assert_eq!(locate(&fixture.path("app/src")), None);
    fixture.write(
        &format!("{store}/{}", &BUNDLE["node_modules/".len()..]),
        b"",
    );
    let beside = locate(&fixture.path("app/src")).unwrap();
    assert!(!beside.to_string_lossy().starts_with(r"\\?\"), "{beside:?}");
    assert_eq!(
        std::fs::canonicalize(&beside).unwrap(),
        std::fs::canonicalize(
            fixture.path(&format!("{store}/{}", &BUNDLE["node_modules/".len()..]))
        )
        .unwrap()
    );
    fixture.write(&format!("app/{BUNDLE}"), b"");
    assert_eq!(
        locate(&fixture.path("app/src")),
        Some(fixture.path(&format!("app/{BUNDLE}")))
    );
}

#[test]
fn lists_project_components_outside_dependencies_and_hidden_directories() {
    let fixture = Fixture::new(&[
        ("src/b.vue", ""),
        ("src/nested/a.vue", ""),
        ("src/script.js", ""),
        ("node_modules/lib/c.vue", ""),
        (".cache/d.vue", ""),
    ]);
    assert_eq!(
        components(fixture.root()),
        vec![fixture.path("src/b.vue"), fixture.path("src/nested/a.vue")]
    );
    assert!(components(&fixture.path("missing")).is_empty());
}

#[test]
fn drivers_import_the_compiler_bundle_by_its_forward_slash_path() {
    let source = driver(Path::new(r"C:\store\compiler.js"));
    assert!(
        source.contains(r#"from "C:/store/compiler.js";"#),
        "{source}"
    );
    assert!(!source.contains("'COMPILER_SFC'"));
}

#[test]
fn component_modules_import_their_stylesheets_after_the_script() {
    let component = parse(r#"{"code":"export default {}","styles":["a{}","b{}"]}"#).unwrap();
    assert_eq!(component.styles, ["a{}", "b{}"]);
    assert_eq!(
        module_source(&component),
        "export default {}\nimport \"nativeboat:vue-style:0\";\nimport \"nativeboat:vue-style:1\";\n"
    );
    assert_eq!(style_index("nativeboat:vue-style:1"), Some(1));
    assert_eq!(style_index("nativeboat:vue-style:x"), None);
    assert_eq!(style_index("./style.css"), None);
    assert_eq!(parse("{"), None);
    assert_eq!(parse(r#"{"styles":[]}"#), None);
    assert_eq!(parse(r#"{"code":"","styles":{}}"#), None);
    assert_eq!(parse(r#"{"code":"","styles":[1]}"#), None);
}

#[test]
fn projects_without_pages_components_or_a_compiler_use_the_built_in_lowering() {
    let fixture = app();
    let cache = fixture.path("cache");
    let args = settings(&fixture);
    let project = nativeboat_project::load(fixture.root()).unwrap();
    // No compiler.
    fixture.write("src/App.vue", b"{}");
    assert_eq!(prepare_in(&project, &args, &cache), Ok(None));
    // No components.
    fixture.write(BUNDLE, STUB.as_bytes());
    std::fs::remove_file(fixture.path("src/App.vue")).unwrap();
    assert_eq!(prepare_in(&project, &args, &cache), Ok(None));
    // No pages.
    fixture.write("src/App.vue", b"{}");
    fixture.write("main.cjs", b"console.log('no windows');");
    std::fs::remove_file(fixture.path("src/index.html")).unwrap();
    let project = nativeboat_project::load(fixture.root()).unwrap();
    assert_eq!(prepare_in(&project, &args, &cache), Ok(None));
    assert!(!cache.exists());
}

#[test]
fn tool_failures_name_their_cause_and_leave_no_partial_build() {
    let fixture = app();
    let args = settings(&fixture);
    let cache = fixture.path("cache");
    let bundle = fixture.path(BUNDLE);
    let missing = tool(&bundle, &args, &cache).unwrap_err();
    assert!(missing.starts_with("cannot read "), "{missing}");

    fixture.write(BUNDLE, b"export const parse = ;");
    fixture.write("cache", b"not a directory");
    let blocked = tool(&bundle, &args, &cache).unwrap_err();
    assert!(blocked.starts_with("cannot create "), "{blocked}");
    std::fs::remove_file(&cache).unwrap();

    let directory = cache.join(key(&bundle));
    std::fs::create_dir_all(directory.join("project/package.json")).unwrap();
    let unwritable = tool(&bundle, &args, &cache).unwrap_err();
    assert!(unwritable.starts_with("cannot write "), "{unwritable}");
    assert!(!directory.exists());

    let invalid = tool(&bundle, &args, &cache).unwrap_err();
    assert!(invalid.contains("compiler-sfc.esm-browser.js"), "{invalid}");
    assert!(!directory.exists());

    // The project reports the failure as a build error.
    fixture.write("src/App.vue", b"{}");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let errors = prepare_in(&project, &args, &cache).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].starts_with("the Vue SFC tool: "), "{errors:?}");

    // A cached tool is used as it is.
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("nativeboat-vue-sfc.exe"), b"").unwrap();
    assert_eq!(
        tool(&bundle, &args, &cache),
        Ok(directory.join("nativeboat-vue-sfc.exe"))
    );
}

#[test]
fn builds_reject_projects_whose_compiler_does_not_build() {
    let fixture = app();
    fixture.write(BUNDLE, b"export const parse = ;");
    fixture.write("src/App.vue", b"{}");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::run(
        [
            "nativeboat".to_owned(),
            "build".to_owned(),
            fixture.root().to_string_lossy().into_owned(),
        ],
        &mut out,
        &mut err,
    );
    let err = String::from_utf8(err).unwrap();
    assert_eq!(code, 1, "{err}");
    assert!(err.starts_with("error: the Vue SFC tool: "), "{err}");
}

#[test]
fn compiles_components_with_the_native_tool_and_caches_each_result() {
    let fixture = app();
    let args = settings(&fixture);
    let cache = fixture.path("cache");
    fixture.write(BUNDLE, STUB.as_bytes());
    fixture.write(
        "src/App.vue",
        br#"{"script":"const _sfc_main = { name: 'App' }","template":"<main/>","styles":[{"content":"a{color:red}","scoped":true},{"content":"b{}"}]}"#,
    );
    fixture.write(
        "src/Setup.vue",
        br#"{"setup":"const _sfc_main = {}","lang":"ts","template":"inline"}"#,
    );
    fixture.write("src/nested/Plain.vue", b"{}");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let compiled = prepare_in(&project, &args, &cache).unwrap().unwrap();
    assert_eq!(compiled.len(), 3);
    let app = &compiled[&fixture.path("src/App.vue")];
    assert!(
        app.code.starts_with(
            "const _sfc_main = { name: 'App' }\n\nfunction _sfc_render() { return \"<main/>\"; }"
        ),
        "{}",
        app.code
    );
    let scope = app.styles[0]
        .strip_prefix("a[data-v-")
        .and_then(|rest| rest.strip_suffix("]{color:red}"))
        .unwrap();
    assert_eq!(scope.len(), 8);
    assert_eq!(app.styles[1], "b{}");
    assert!(app.code.ends_with(&format!(
        "export default /*#__PURE__*/_export_sfc(_sfc_main, [['render',_sfc_render],['__scopeId',\"data-v-{scope}\"]])"
    )));
    assert_eq!(
        compiled[&fixture.path("src/Setup.vue")].code,
        "const _sfc_main = {} /* inline */\nexport default _sfc_main"
    );
    assert_eq!(
        compiled[&fixture.path("src/nested/Plain.vue")],
        Compiled {
            code: "const _sfc_main = {}\nexport default _sfc_main".to_owned(),
            styles: Vec::new(),
        }
    );

    // Unchanged components come from the cache without running the tool.
    let tool = cache
        .join(key(&fixture.path(BUNDLE)))
        .join("nativeboat-vue-sfc.exe");
    let away = cache.join("away.exe");
    std::fs::rename(&tool, &away).unwrap();
    assert_eq!(
        prepare_in(&project, &args, &cache).unwrap().unwrap(),
        compiled
    );
    std::fs::rename(&away, &tool).unwrap();

    let root = fixture.path("src");
    let failing = |name: &str, spec: &str| {
        fixture.write(name, spec.as_bytes());
        compile(&tool, &root, &[fixture.path(name)], &cache).unwrap_err()
    };
    let errors = failing(
        "src/Broken.vue",
        r#"{"parseErrors":["bad", {"message":"worse"}]}"#,
    );
    let broken = fixture.path("src/Broken.vue");
    assert_eq!(
        errors,
        [
            format!("{}:1:1: bad", broken.display()),
            format!("{}:1:1: worse", broken.display())
        ]
    );
    let messages = |name: &str, spec: &str| {
        failing(name, spec)[0]
            .split_once(": ")
            .unwrap()
            .1
            .to_owned()
    };
    assert_eq!(
        messages("src/T.vue", r#"{"template":"bad"}"#),
        "bad template"
    );
    assert_eq!(
        messages("src/S.vue", r#"{"styles":[{"content":"bad{}"}]}"#),
        "bad style"
    );
    assert_eq!(
        messages("src/L.vue", r#"{"styles":[{"content":"","lang":"scss"}]}"#),
        "<style> block 0 uses lang=\"scss\", which native builds do not compile yet"
    );
    assert_eq!(
        messages("src/M.vue", r#"{"styles":[{"content":"","module":true}]}"#),
        "<style> block 0 uses module, which native builds do not compile yet"
    );
    assert_eq!(
        messages(
            "src/R.vue",
            r#"{"styles":[{"content":"","src":"./a.css"}]}"#
        ),
        "<style> block 0 uses src, which native builds do not compile yet"
    );
    assert_eq!(messages("src/X.vue", r#"{"throw":"thrown"}"#), "thrown");
    assert_eq!(
        messages("src/N.vue", r#"{"write":"{}"}"#),
        "the Vue SFC tool returned no module"
    );
    let failure = |spec: &str| failing("src/F.vue", spec);
    assert_eq!(
        failure(r#"{"exit":3}"#),
        ["the Vue SFC tool failed (exit code: 3)"]
    );
    assert!(failure(r#"{"exit":0}"#)[0].starts_with("cannot read the output of "));
    assert!(failure(r#"{"write":"garbage"}"#)[0].starts_with("invalid Vue SFC tool output: "));

    let file = [fixture.path("src/F.vue")];
    let missing = compile(&fixture.path("missing.exe"), &root, &file, &cache).unwrap_err();
    assert!(missing[0].starts_with("cannot run "), "{missing:?}");
    let unread = compile(&tool, &root, &[fixture.path("src/Gone.vue")], &cache).unwrap_err();
    assert!(unread[0].starts_with("cannot read "), "{unread:?}");
    fixture.write("blocked/vue-components", b"not a directory");
    let blocked = compile(&tool, &root, &file, &fixture.path("blocked")).unwrap_err();
    assert!(blocked[0].starts_with("cannot create "), "{blocked:?}");
    let request = cache
        .join("vue-components")
        .join(format!("request-{}.json", std::process::id()));
    std::fs::create_dir_all(&request).unwrap();
    let unwritable = compile(&tool, &root, &file, &cache).unwrap_err();
    assert!(unwritable[0].starts_with("cannot write "), "{unwritable:?}");
}
