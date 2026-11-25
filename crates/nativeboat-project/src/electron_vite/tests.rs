use super::*;
use crate::tests::fixture::Fixture;
use nativeboat_bundler::Target;
use std::collections::BTreeMap;

fn resolve(fixture: &Fixture, main: &str) -> Result<Entries, String> {
    discover(
        fixture.root(),
        &Resolver {
            root: fixture.root().to_path_buf(),
            target: Target::Main,
            aliases: Vec::new(),
        },
        main,
    )
}

#[test]
fn renderer_plugins_css_and_asset_configuration_require_the_project_tool() {
    for (renderer, expected) in [
        ("{}", false),
        ("{plugins:[]}", false),
        ("{plugins:[vue()]}", true),
        ("{css:{postcss:{plugins:[nesting()]}}}", true),
        ("{assetsInclude:['**/*.md']}", true),
    ] {
        let config = format!("export default {{renderer:{renderer}}}");
        let fixture = Fixture::new(&[("electron.vite.config.ts", &config)]);
        assert_eq!(
            resolve(&fixture, "main.js").unwrap().renderer_transforms,
            expected,
            "{renderer}"
        );
    }
}

#[test]
fn configured_renderer_output_is_fresh_portable_and_checked_before_replacement() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"out/main/index.js","devDependencies":{"electron":"44","electron-vite":"5"}}"#,
        ),
        (
            "electron.vite.config.ts",
            "export default {renderer:{root:'ui',plugins:[vue()],build:{outDir:'release/renderer',rollupOptions:{input:['ui/nested/index.html','outside.html']}}}}",
        ),
        ("src/main/index.js", "require('electron');"),
        ("ui/nested/index.html", "<p>source</p>"),
        ("outside.html", "<p>outside</p>"),
        ("fresh/renderer/nested/index.html", "<p>prepared</p>"),
        ("fresh/renderer/outside.html", "<p>prepared outside</p>"),
    ]);
    let mut project = crate::load(fixture.root()).unwrap();
    assert_eq!(
        project.renderer_build.as_ref().unwrap().runtime_root,
        "release/renderer"
    );
    let main = project.main.clone();
    let source = project.pages.clone();
    assert!(
        project
            .apply_renderer_output(&fixture.path("missing"))
            .unwrap_err()
            .contains("did not produce")
    );
    assert_eq!(project.pages, source);
    project
        .apply_renderer_output(&fixture.path("fresh"))
        .unwrap();
    assert_eq!(project.main, main);
    assert_eq!(
        project.pages[0].source,
        fixture.path("fresh/renderer/nested/index.html")
    );
    assert_eq!(
        project.pages[1].source,
        fixture.path("fresh/renderer/outside.html")
    );
    assert_eq!(
        project.pages[0].runtime_paths,
        ["release/renderer/nested/index.html"]
    );
    assert_eq!(
        project.runtime_path(&fixture.path("fresh/renderer/assets/icon.svg")),
        "release/renderer/assets/icon.svg"
    );
    assert_eq!(
        project.runtime_path(&fixture.path("src/main/index.js")),
        "src/main/index.js"
    );
    project.renderer_build = None;
    project
        .apply_renderer_output(&fixture.path("missing"))
        .unwrap();
}

#[test]
fn custom_electron_vite_entries_do_not_require_a_prior_build() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"release/app/dist/main/main.js","devDependencies":{"electron":"44","electron-vite":"5"}}"#,
        ),
        (
            "electron.vite.config.ts",
            r#"
import path from 'node:path'; import {defineConfig} from 'electron-vite';
const root = __dirname; const external = Object.keys(dependencies || {});
export default defineConfig({
  main: {build:{outDir:'release/app/dist/main',rollupOptions:{
    input:path.join(root,'src','main','main.ts'),external,
    output:{entryFileNames:'main.js'}}}},
  preload: {build:{outDir:'release/app/dist/preload',rollupOptions:{
    input:path.resolve(root,'src/main/preload.ts'),output:{entryFileNames:'preload.js'}}}},
  renderer: {root:path.join(root,'src/renderer'),build:{
    outDir:path.join(root,'release/app/dist/renderer'),
    rollupOptions:{input:path.join(root,'src/renderer/index.html')}}}
});"#,
        ),
        ("src/main/main.ts", "import {app} from 'electron';"),
        (
            "src/main/preload.ts",
            "import {contextBridge} from 'electron';",
        ),
        (
            "src/main/index.ts",
            "throw new Error('wrong conventional source');",
        ),
        (
            "release/app/dist/main/main.js",
            "throw new Error('stale bundle');",
        ),
        (
            "src/renderer/index.html",
            "<script type=module src=/index.tsx></script>",
        ),
        ("src/renderer/unused.html", ""),
        ("src/renderer/index.tsx", ""),
    ]);
    let project = crate::load(fixture.root()).unwrap();
    assert_eq!(project.main.source, fixture.path("src/main/main.ts"));
    assert_eq!(
        project.main.runtime_paths,
        ["release/app/dist/main/main.js"]
    );
    assert_eq!(
        project.preloads[0].source,
        fixture.path("src/main/preload.ts")
    );
    assert_eq!(
        project.preloads[0].runtime_paths,
        ["release/app/dist/preload/preload.js"]
    );
    assert_eq!(project.pages.len(), 1);
    assert_eq!(
        project.pages[0].runtime_paths,
        ["release/app/dist/renderer/index.html"]
    );
    assert_eq!(project.renderer_root, fixture.path("src/renderer"));
    assert!(project.build_steps.is_empty());
}

#[test]
fn named_and_array_inputs_preserve_nested_html_outputs() {
    let fixture = Fixture::new(&[
        (
            "electron.vite.config.cts",
            r#"
const ignored = undefined; let uninitialized; const {a} = unknown;
const base = {outDir:'compiled'}; const name = 'named';
const main = {build:{...base,lib:{entry:{worker:'process/worker.ts',named:'process/run.ts'}},
  rolldownOptions:{output:[{entryFileNames:'entry-[name].cjs'}]}}};
module.other = 1; exports.other = 1; global.other(); other = 2;
module.exports = defineConfig(() => ({main,
  preload: {build:{outDir:'bridges',rollupOptions:{input:['bridge/a.ts','bridge/b.ts']}}},
  renderer: {root:'ui',build:{outDir:'pages',rollupOptions:{input:{index:'ui/index.html',about:'ui/settings/about.html',outside:'other.html'}}}}
}) satisfies UserConfig);"#,
        ),
        ("process/run.ts", ""),
        ("process/worker.ts", ""),
        ("bridge/a.ts", ""),
        ("bridge/b.ts", ""),
        ("ui/index.html", ""),
        ("ui/settings/about.html", ""),
        ("other.html", ""),
    ]);
    let entries = resolve(&fixture, "compiled/entry-named.cjs").unwrap();
    assert_eq!(entries.main.unwrap().source, fixture.path("process/run.ts"));
    assert_eq!(entries.preloads.len(), 2);
    assert_eq!(
        entries.preloads[1].runtime_paths,
        ["bridges/b.js", "bridges/b.mjs"]
    );
    assert!(
        entries
            .pages
            .iter()
            .any(|entry| entry.runtime_paths == ["pages/settings/about.html"])
    );
    assert!(
        entries
            .pages
            .iter()
            .any(|entry| entry.runtime_paths == ["pages/other.html"])
    );
    assert!(
        resolve(&fixture, "compiled/absent.js")
            .unwrap_err()
            .contains("does not select")
    );
}

#[test]
fn default_entries_include_main_and_preload_filenames_and_all_config_extensions() {
    for config_name in CONFIGS {
        let fixture = Fixture::new(&[
            (
                config_name,
                "export default defineConfig(() => { return {}; });",
            ),
            ("src/main/main.cjs", ""),
            ("src/preload/preload.mts", ""),
            ("src/renderer/index.html", ""),
        ]);
        let entries = resolve(&fixture, "out/main/main.js").unwrap();
        assert_eq!(
            entries.main.unwrap().source,
            fixture.path("src/main/main.cjs")
        );
        assert_eq!(
            entries.preloads[0].runtime_paths,
            ["out/preload/preload.js", "out/preload/preload.mjs"]
        );
    }
    let fixture = Fixture::new(&[
        ("src/main/index.ts", ""),
        ("src/preload/index.js", ""),
        ("src/renderer/index.html", ""),
    ]);
    assert!(
        resolve(&fixture, "out/main/index.js")
            .unwrap()
            .main
            .is_some()
    );
    assert!(
        resolve(&Fixture::new(&[]), "index.js")
            .unwrap()
            .main
            .is_none()
    );
}

#[test]
fn unresolved_configuration_is_reported_instead_of_executing_or_guessing() {
    let typed = Fixture::new(&[(
        "electron.vite.config.ts",
        "export default ({} as UserConfig);",
    )]);
    assert!(resolve(&typed, "main.js").is_ok());
    for source in [
        "export default defineConfig()",
        "export default defineConfig(...unknown)",
        "export default function config() { return {}; }",
        "export default defineConfig(() => {})",
        "export default defineConfig(() => { return; })",
        "export default defineConfig(() => { throw new Error('no execution'); })",
        "export default defineConfig(() => { run(); return {}; })",
        "export default {...unknown}",
        "export default Object['assign']({}, {})",
        "export default defineConfig({",
    ] {
        let fixture = Fixture::new(&[("electron.vite.config.ts", source)]);
        assert!(resolve(&fixture, "main.js").is_err(), "{source}");
    }
    for (source, expected) in [
        ("export default {main:choose()}", "main"),
        ("export default {preload:{build:unknown}}", "preload.build"),
        (
            "export default {main:{build:{rollupOptions:unknown}}}",
            "rollupOptions",
        ),
        (
            "export default {main:{build:{rolldownOptions:unknown}}}",
            "rolldownOptions",
        ),
        (
            "export default {main:{build:{rollupOptions:{input:choose()}}}}",
            "build input",
        ),
        (
            "export default {main:{build:{rollupOptions:{input:path.join(unknown,'main.ts')}}}}",
            "build input",
        ),
        (
            "export default {main:{build:{rollupOptions:{input:[,'main.ts']}}}}",
            "build input",
        ),
        (
            "export default {main:{build:{rollupOptions:{input:[...unknown]}}}}",
            "build input",
        ),
        ("export default {main:{build:{outDir:42}}}", "outDir"),
        (
            "export default {main:{build:{rollupOptions:{input:'missing.ts'}}}}",
            "main input",
        ),
        ("export default {renderer:{root:unknown}}", "renderer.root"),
        (
            "export default {renderer:{build:{outDir:unknown}}}",
            "renderer.build.outDir",
        ),
        (
            "export default {renderer:{build:{rollupOptions:{input:'missing.html'}}}}",
            "renderer HTML input",
        ),
        (
            "export default {renderer:{build:{rollupOptions:{input:'main.ts'}}}}",
            "renderer HTML input",
        ),
        (
            "export default {main:{build:{rollupOptions:{input:'main.ts',output:{entryFileNames:fn}}}}}",
            "entryFileNames",
        ),
        (
            "export default {main:{build:{rollupOptions:{input:'main.ts',output:{entryFileNames:'[name]-[hash].js'}}}}}",
            "output name",
        ),
    ] {
        let fixture = Fixture::new(&[("electron.vite.config.ts", source), ("main.ts", "")]);
        assert!(
            resolve(&fixture, "main.js").unwrap_err().contains(expected),
            "{source}"
        );
    }
}

#[test]
fn failed_config_reads_are_reported() {
    let fixture = Fixture::new(&[("electron.vite.config.js", "")]);
    let mut session = shimforge::Session::new();
    let read = shimforge::mock!(
        session,
        std::fs::read_to_string::<&PathBuf>,
        fn(&PathBuf) -> std::io::Result<String>
    );
    read.expect()
        .returning(|_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
    assert!(
        resolve(&fixture, "main.js")
            .unwrap_err()
            .contains("cannot read")
    );
}

#[test]
fn static_defines_use_json_metadata_and_remain_separate_for_each_process() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"version":"1.2.3","flags":[true,null,42]}"#,
        ),
        (
            "electron.vite.config.ts",
            r#"import pkg from './package.json' with {type:'json'};
import {version as release} from './package.json';
import * as namespace from './package.json';
import type {Unused} from './missing.json';
const shared = {FLAG:true, NIL:null};
export default {main:{define:{...shared,VERSION:JSON.stringify(pkg.version),LABEL:JSON.stringify(`v${release}`),META:JSON.stringify(namespace.default.flags),NAME:JSON.stringify(namespace.version)}},
preload:{define:{CHANNEL:JSON.stringify('preload')}},renderer:{define:{global:'globalThis',FLAG:false}}};"#,
        ),
    ]);
    let entries = resolve(&fixture, "main.js").unwrap();
    let definitions = |index: usize| {
        entries.defines[index]
            .iter()
            .cloned()
            .collect::<BTreeMap<_, _>>()
    };
    let main = definitions(0);
    assert_eq!(main["VERSION"], r#""1.2.3""#);
    assert_eq!(main["LABEL"], r#""v1.2.3""#);
    assert_eq!(main["META"], "[true,null,42]");
    assert_eq!(main["NAME"], r#""1.2.3""#);
    assert_eq!(main["FLAG"], "true");
    assert_eq!(main["NIL"], "null");
    assert_eq!(definitions(1)["CHANNEL"], r#""preload""#);
    assert_eq!(definitions(2)["global"], "globalThis");
    assert_eq!(definitions(2)["FLAG"], "false");
    assert!(!definitions(1).contains_key("VERSION"));
    fixture.write("package.json", b"42");
    assert!(
        resolve(&fixture, "main.js")
            .unwrap_err()
            .contains("main.define.VERSION")
    );
}

#[test]
fn dynamic_defines_and_broken_json_imports_have_actionable_diagnostics() {
    let fixture = Fixture::new(&[]);
    for (source, expected) in [
        ("export default {main:{define:choose()}}", "main.define"),
        (
            "export default {main:{define:{VERSION:unknown}}}",
            "main.define.VERSION",
        ),
        (
            "export default {preload:{define:{FLAG:choose()}}}",
            "preload.define.FLAG",
        ),
        (
            "export default {renderer:{define:{global:choose()}}}",
            "renderer.define.global",
        ),
        (
            "export default {main:{define:{[choose()]:true}}}",
            "main.define",
        ),
        (
            "import pkg from './missing.json'; export default {}",
            "cannot read",
        ),
    ] {
        fixture.write("electron.vite.config.ts", source.as_bytes());
        assert!(
            resolve(&fixture, "main.js").unwrap_err().contains(expected),
            "{source}"
        );
    }
    fixture.write("broken.json", b"{");
    fixture.write(
        "electron.vite.config.ts",
        b"import pkg from './broken.json'; export default {}",
    );
    assert!(
        resolve(&fixture, "main.js")
            .unwrap_err()
            .contains("invalid static JSON import")
    );
}

#[test]
fn esm_paths_support_renamed_and_namespace_imports_without_executing_config() {
    let fixture = Fixture::new(&[
        (
            "space #项目/package.json",
            r#"{"main":"out/main/custom.js","devDependencies":{"electron":"1"}}"#,
        ),
        (
            "space #项目/electron.vite.config.mts",
            r#"
import {resolve as absolute, dirname as directory} from 'node:path';
import {fileURLToPath as filename, URL as FileUrl} from 'node:url';
import * as paths from 'path';
import urls from 'url';
import {defineConfig as configuration} from 'electron-vite';
import type {Ignored} from 'path';
const __filename = filename(import.meta.url);
const __dirname = directory(__filename);
export default configuration({
 main:{build:{rollupOptions:{input:{custom:absolute(__dirname, 'process.ts')}}},define:{CONFIG:JSON.stringify(__filename)}},
 preload:{build:{rollupOptions:{input:filename(new FileUrl('./bridge.ts', import.meta.url))}}},
 renderer:{root:paths.resolve(__dirname, 'ui'),build:{rollupOptions:{input:urls.fileURLToPath(new URL('./ui/index.html',import.meta.url))}}}
});"#,
        ),
        ("space #项目/process.ts", ""),
        ("space #项目/bridge.ts", ""),
        ("space #项目/ui/index.html", ""),
    ]);
    let root = fixture.path("space #项目");
    let project = crate::load(&root).unwrap();
    assert_eq!(project.main.source, root.join("process.ts"));
    assert_eq!(project.main.runtime_paths, ["out/main/custom.js"]);
    assert_eq!(project.preloads[0].source, root.join("bridge.ts"));
    assert_eq!(project.pages[0].source, root.join("ui/index.html"));
    let config: String = serde_json::from_str(&project.defines[0].1).unwrap();
    assert_eq!(Path::new(&config), root.join("electron.vite.config.mts"));
}

#[test]
fn similarly_named_or_shadowed_config_functions_are_not_assumed_to_be_node_helpers() {
    for source in [
        "import {resolve} from './custom.js'; export default {main:{build:{rollupOptions:{input:resolve('main.ts')}}}}",
        "import path from 'node:path'; const resolve = custom; export default {main:{build:{rollupOptions:{input:resolve('main.ts')}}}}",
        "import {fileURLToPath} from 'url'; export default {main:{build:{rollupOptions:{input:fileURLToPath('https://example.org/main.ts')}}}}",
        "import {unknown} from 'path'; export default {main:{build:{rollupOptions:{input:unknown('main.ts')}}}}",
    ] {
        let fixture = Fixture::new(&[("electron.vite.config.ts", source)]);
        assert!(
            resolve(&fixture, "main.js")
                .unwrap_err()
                .contains("build input"),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires an unchanged MarkText checkout; set NATIVEBOAT_MARKTEXT_PROJECT"]
fn unchanged_marktext_esm_configuration_paths_are_evaluated() {
    let root = PathBuf::from(std::env::var_os("NATIVEBOAT_MARKTEXT_PROJECT").unwrap())
        .join("packages/desktop");
    let data = config(&root).unwrap().data;
    for scope in ["main", "preload", "renderer"] {
        for (alias, path) in [
            ("@", "src/renderer/src"),
            ("common", "src/common"),
            ("@shared", "src/shared"),
        ] {
            assert_eq!(
                data[scope]["resolve"]["alias"][alias],
                root.join(path).to_string_lossy().replace('\\', "/")
            );
        }
    }
    assert_eq!(data["renderer"]["resolve"]["alias"]["path"], "pathe");
    println!(
        "MarkText: ESM directory construction and static path aliases evaluated without config or plugin execution"
    );
}

#[test]
fn static_aliases_keep_process_scope_and_first_rule_order() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"out/main/index.js","devDependencies":{"electron":"1"}}"#,
        ),
        (
            "electron.vite.config.ts",
            r#"
import {resolve} from 'node:path';
const mainAliases={phase:resolve(__dirname,'main-lib'),path:'main-path'};
const rendererAliases=[{find:'phase',replacement:resolve(__dirname,'ui-lib')},{find:'phase/deep',replacement:resolve(__dirname,'later-rule')}];
export default {
 main:{resolve:{alias:mainAliases}}, preload:{resolve:{alias:{phase:resolve(__dirname,'bridge-lib')}}},
 renderer:{resolve:{alias:rendererAliases}}
};"#,
        ),
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"paths":{"phase":["fallback"],"shared":["shared"]}}}"#,
        ),
        ("src/main/index.ts", "import 'phase';"),
        ("src/preload/index.ts", ""),
        ("src/renderer/index.html", ""),
        ("main-lib/index.js", ""),
    ]);
    let project = crate::load(fixture.root()).unwrap();
    for (aliases, expected) in [
        (&project.aliases, "main-lib"),
        (&project.preload_aliases, "bridge-lib"),
        (&project.renderer_aliases, "ui-lib"),
    ] {
        assert_eq!(aliases[0].pattern, "phase");
        assert_eq!(
            aliases[0].target,
            fixture.path(expected).to_string_lossy().replace('\\', "/")
        );
        assert_eq!(aliases[0].kind, AliasKind::Path);
        assert!(aliases.iter().any(|alias| alias.pattern == "shared"));
    }
    assert_eq!(project.aliases[2].target, "main-path");
    assert_eq!(project.aliases[2].kind, AliasKind::Specifier);
    assert!(
        !project
            .renderer_aliases
            .iter()
            .any(|alias| alias.pattern == "path")
    );
    assert_eq!(project.renderer_aliases[1].pattern, "phase/*");
    assert_eq!(project.renderer_aliases[2].pattern, "phase/deep");
}

#[test]
fn unresolved_alias_rules_are_reported_in_the_relevant_process() {
    for (source, setting) in [
        ("export default {main:{resolve:unknown}}", "main.resolve"),
        (
            "export default {main:{resolve:{alias:[{find:'phase',replacement:'lib',customResolver:choose()}]}}}",
            "unsupported custom resolver",
        ),
        (
            "export default {main:{resolve:{alias:unknown}}}",
            "main.resolve.alias",
        ),
        (
            "export default {preload:{resolve:{alias:{phase:unknown}}}}",
            "preload.resolve.alias entry",
        ),
        (
            "export default {renderer:{resolve:{alias:[{find:/^phase/,replacement:'lib'}]}}}",
            "renderer.resolve.alias entry",
        ),
        (
            "export default {renderer:{resolve:{alias:[{find:'phase'}]}}}",
            "renderer.resolve.alias entry",
        ),
        (
            "export default {renderer:{resolve:{alias:[42]}}}",
            "renderer.resolve.alias entry",
        ),
        (
            "export default {main:{resolve:{alias:{'phase*':'lib'}}}}",
            "unsupported string pattern",
        ),
        (
            "export default {main:{resolve:{alias:{phase:'lib*'}}}}",
            "unsupported string pattern",
        ),
        (
            "export default {main:{resolve:{alias:{'':'lib'}}}}",
            "unsupported string pattern",
        ),
    ] {
        let fixture = Fixture::new(&[("electron.vite.config.ts", source)]);
        assert!(
            resolve(&fixture, "main.js").unwrap_err().contains(setting),
            "{source}"
        );
    }
    for source in [
        "export default {}",
        "export default {main:{resolve:{extensions:['.ts']}}}",
        "export default {main:{resolve:{alias:[]}}}",
    ] {
        let fixture = Fixture::new(&[("electron.vite.config.ts", source)]);
        assert!(
            resolve(&fixture, "main.js")
                .unwrap()
                .aliases
                .iter()
                .all(Vec::is_empty)
        );
    }
}
