use super::*;
use crate::tests::fixture::Fixture;
use nativeboat_compiler::compile_sources;
use std::os::windows::process::{CommandExt, ExitStatusExt};

#[test]
fn every_native_runtime_source_is_embedded() {
    // A module the runtime declares but the compiler does not embed fails
    // only when an application is built, hours later.
    fn walk(root: &std::path::Path, dir: &std::path::Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "tests" {
                    walk(root, &path, found);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" {
                let relative = path.strip_prefix(root).unwrap();
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../nativeboat-native/src");
    let mut found = Vec::new();
    walk(&root, &root, &mut found);
    let embedded: Vec<&str> = SOURCES.iter().map(|(name, _)| *name).collect();
    let missing: Vec<&String> = found
        .iter()
        .filter(|name| !embedded.contains(&name.as_str()))
        .collect();
    assert!(missing.is_empty(), "not embedded: {missing:?}");
}

#[test]
fn cached_fixture_runtimes_cannot_replace_production_optimization() {
    let key = 12345;
    assert_eq!(runtime_optimization_key(key, 2), key);
    let fixture = runtime_optimization_key(key, 0);
    assert_ne!(fixture, key);
    assert_ne!(fixture, runtime_optimization_key(key, 1));
    assert_ne!(fixture, runtime_optimization_key(key + 1, 0));
}

#[test]
fn concurrent_build_leases_protect_libraries_until_the_last_link_finishes() {
    let fixture = Fixture::new(&[]);
    let cache = fixture.path("cache");
    let first = CacheUse::new(&cache).unwrap();
    let second = CacheUse::new(&cache).unwrap();
    for name in ["generated_renderer-old", "generated_renderer-new"] {
        let (directory, _) = cached(&cache, name, "lib.rlib", |staging| {
            write_file(&staging.join("lib.rlib"), b"compiled library")
                .map_err(|error| error.to_string())
        })
        .unwrap();
        assert!(directory.join("lib.rlib").is_file());
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let old = cache.join("generated_renderer-old/lib.rlib");
    let new = cache.join("generated_renderer-new/lib.rlib");
    assert!(old.is_file() && new.is_file());
    drop(first);
    assert!(old.is_file() && new.is_file());
    drop(second);
    assert!(!old.exists() && new.is_file());
    fixture.write("blocked-cache", b"not a directory");
    assert!(CacheUse::new(&fixture.path("blocked-cache")).is_err());
}

/// A chunk whose cache entry was removed compiles again to the same bytes,
/// so the libraries depending on it keep their keys, and the program runs.
#[test]
fn rebuilding_an_evicted_chunk_reproduces_it_for_dependent_native_libraries() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-cache-rebuild","main":"main.cjs","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.cjs",
            "'use strict'; console.log(require('./large.cjs'));",
        ),
    ]);
    let large = format!(
        "'use strict'; module.exports = 7; const padding = '{}';",
        "x".repeat(2 * 1024 * 1024)
    );
    fixture.write("large.cjs", large.as_bytes());
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-cache-rebuild.exe");
    assert_eq!(Command::new(&executable).output().unwrap().stdout, b"7\n");
    let scope = generated_cache_scope(&fixture.path("native-output/staging"));
    let prefix = format!("generated_{scope:016x}_nativeboat_main_0_chunk_1-");
    let cache = cache_directory();
    let leaf = std::fs::read_dir(&cache)
        .unwrap()
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .unwrap()
        .path();
    assert_eq!(leaf.parent(), Some(cache.as_path()));
    let library = leaf.join("libnativeboat_main_0_chunk_1.rlib");
    let before = artifact_fingerprint(&library).unwrap();
    std::fs::remove_dir_all(&leaf).unwrap();
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(artifact_fingerprint(&library).unwrap(), before);
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"7\n");
    assert!(executed.stderr.is_empty());
}

/// A cached library folder rebuilt in place, as after the cache pruned it,
/// is another dependency to what links it, so the runtime compiled against
/// the old copy is not reused; other files do not count.
#[test]
fn library_folders_are_identified_by_their_rlibs() {
    let fixture = Fixture::new(&[("libdep.rlib", "first"), ("dep/notes.txt", "a")]);
    let folder = fixture.path("");
    let before = libraries_fingerprint(&folder).unwrap();
    fixture.write("dep/notes.txt", b"b");
    fixture.write(".last-use", b"");
    assert_eq!(libraries_fingerprint(&folder).unwrap(), before);
    fixture.write("libdep.rlib", b"second");
    assert_ne!(libraries_fingerprint(&folder).unwrap(), before);
    assert_ne!(
        libraries_fingerprint(&fixture.path("dep")).unwrap(),
        libraries_fingerprint(&fixture.path("missing")).unwrap()
    );
}

#[test]
fn generated_cache_scopes_and_dependency_artifacts_are_independent() {
    let project = Path::new("C:/project/output/.staging-1");
    assert_eq!(
        generated_cache_scope(project),
        generated_cache_scope(Path::new("C:/project/output/.staging-2"))
    );
    assert_ne!(
        generated_cache_scope(project),
        generated_cache_scope(Path::new("C:/other/output/.staging-1"))
    );
    assert_ne!(
        generated_dependency_fingerprint(7, &[11]),
        generated_dependency_fingerprint(7, &[12])
    );
    assert_eq!(generated_dependency_fingerprint(7, &[]), 7);
}

#[test]
fn generated_cache_entries_are_shared_by_spellings_of_one_owner() {
    let fixture = Fixture::new(&[]);
    let output = fixture.path("output");
    let other = fixture.path("other");
    std::fs::create_dir(&output).unwrap();
    std::fs::create_dir(&other).unwrap();
    let slashed = PathBuf::from(output.to_string_lossy().replace('\\', "/")).join(".staging");
    assert_eq!(
        generated_cache_scope(&output.join(".staging")),
        generated_cache_scope(&slashed)
    );

    let cache = fixture.path("cache");
    let entry = |name: &str, owner: Option<&Path>| {
        let directory = cache.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        if let Some(owner) = owner {
            write_file(
                &directory.join("owner-directory.txt"),
                owner.to_string_lossy().as_bytes(),
            )
            .unwrap();
        }
        directory
    };
    let alias = entry("generated_00000000000000bb_app_chunk_1-01", Some(&slashed));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let current = entry("generated_00000000000000aa_app_chunk_1-02", Some(&output));
    let legacy = entry("generated_app_chunk_1-01", None);
    let elsewhere = entry("generated_00000000000000cc_app_chunk_1-01", Some(&other));
    let sibling = entry("generated_00000000000000bb_app_chunk_10-01", Some(&output));
    // Starting a build removes them while no other build uses the cache.
    drop(CacheUse::new(&cache).unwrap());
    assert!(current.exists());
    assert!(!alias.exists());
    assert!(!legacy.exists());
    assert!(elsewhere.exists());
    assert!(sibling.exists());
}

#[test]
fn generated_cache_cleanup_reclaims_deleted_output_directories() {
    let fixture = Fixture::new(&[]);
    let cache = fixture.path("cache");
    let output = fixture.path("temporary-output");
    std::fs::create_dir(&output).unwrap();
    let lease = CacheUse::new(&cache).unwrap();
    let (artifact, _) = cached(
        &cache,
        "generated_deleted-output-1",
        "lib.rlib",
        |staging| {
            write_file(&staging.join("lib.rlib"), b"native library")
                .map_err(|error| error.to_string())?;
            write_file(
                &staging.join("owner-directory.txt"),
                output.to_string_lossy().as_bytes(),
            )
            .map_err(|error| error.to_string())
        },
    )
    .unwrap();
    assert!(artifact.join("lib.rlib").exists());
    std::fs::remove_dir(&output).unwrap();
    drop(lease);
    assert!(!artifact.exists());
}

#[test]
fn copies_electron_resources_under_the_output_tree() {
    let fixture = Fixture::new(&[]);
    fixture.write("project/build/icons/icon.ico", b"icon-data");
    fixture.write("project/static/locales/en.json", b"{\"name\":\"English\"}");
    let output = fixture.path("output");
    let files = [
        nativeboat_project::builder::ExtraFile {
            source: fixture.path("project/build/icons/icon.ico"),
            destination: "resources/icons/icon.ico".into(),
        },
        nativeboat_project::builder::ExtraFile {
            source: fixture.path("project/static/locales/en.json"),
            destination: "resources/static/locales/en.json".into(),
        },
    ];

    copy_extra_files(&files, &output).unwrap();

    assert_eq!(
        std::fs::read(output.join("resources/icons/icon.ico")).unwrap(),
        b"icon-data"
    );
    assert_eq!(
        std::fs::read(output.join("resources/static/locales/en.json")).unwrap(),
        b"{\"name\":\"English\"}"
    );
}

#[test]
fn native_build_runs_packaging_steps_and_ships_what_they_make() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"},
               "scripts":{"generate":"mkdir generated & echo made> generated\\data.txt",
                          "package":"npm run generate && tsc && electron-builder --win"},
               "build":{"extraResources":[{"from":"generated","to":"data"}]}}"#,
        ),
        ("main.ts", "console.log('ran');"),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    // The step ran; the JavaScript compiler's did not.
    assert!(out.contains("> generate: mkdir generated"), "{out}");
    assert!(!out.contains("> package: tsc"), "{out}");
    assert_eq!(
        std::fs::read_to_string(fixture.path("native-output/resources/data/data.txt"))
            .unwrap()
            .trim(),
        "made"
    );

    // A failing step stops the build before compiling.
    fixture.write(
        "package.json",
        br#"{"name":"native-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"},
            "scripts":{"package":"exit 3 && electron-builder --win"}}"#,
    );
    let (code, _, err) = run(&fixture, &["--out", "failed-output"]);
    assert_eq!(code, 1);
    assert!(err.contains("failed with exit code 3"), "{err}");
    assert!(!fixture.path("failed-output").exists());
}

#[test]
fn rejects_resource_destinations_that_escape_the_output_tree() {
    let fixture = Fixture::new(&[]);
    fixture.write("icon.ico", b"icon-data");
    let error = copy_extra_files(
        &[nativeboat_project::builder::ExtraFile {
            source: fixture.path("icon.ico"),
            destination: "resources/..\\outside.ico".into(),
        }],
        &fixture.path("output"),
    )
    .unwrap_err();
    assert!(
        error.contains("invalid native resource destination"),
        "{error}"
    );
    assert!(!fixture.path("outside.ico").exists());

    let absolute = copy_extra_files(
        &[nativeboat_project::builder::ExtraFile {
            source: fixture.path("icon.ico"),
            destination: "C:\\outside.ico".into(),
        }],
        &fixture.path("output"),
    )
    .unwrap_err();
    assert!(absolute.contains("invalid native resource destination"));
}

#[test]
fn reports_missing_native_resource_sources() {
    let fixture = Fixture::new(&[]);
    let error = copy_extra_files(
        &[nativeboat_project::builder::ExtraFile {
            source: fixture.path("missing.ico"),
            destination: "resources/missing.ico".into(),
        }],
        &fixture.path("output"),
    )
    .unwrap_err();
    assert!(error.contains("cannot read native resource"), "{error}");

    let directory_source = copy_extra_files(
        &[nativeboat_project::builder::ExtraFile {
            source: fixture.root().to_path_buf(),
            destination: "resources/directory".into(),
        }],
        &fixture.path("output"),
    )
    .unwrap_err();
    assert!(directory_source.contains("is not a regular file"));
}

#[test]
fn reports_native_resource_output_and_copy_failures() {
    let fixture = Fixture::new(&[]);
    fixture.write("source.ico", b"icon-data");
    fixture.write("blocked", b"file blocks directory creation");

    let directory_error = copy_extra_files(
        &[nativeboat_project::builder::ExtraFile {
            source: fixture.path("source.ico"),
            destination: "resources/icon.ico".into(),
        }],
        &fixture.path("blocked"),
    )
    .unwrap_err();
    assert!(directory_error.contains("cannot create native resource directory"));

    fixture.write(
        "output/resources/icon.ico/child",
        b"directory blocks file copy",
    );
    let copy_error = copy_extra_files(
        &[nativeboat_project::builder::ExtraFile {
            source: fixture.path("source.ico"),
            destination: "resources/icon.ico".into(),
        }],
        &fixture.path("output"),
    )
    .unwrap_err();
    assert!(copy_error.contains("cannot copy native resource"));
}

#[test]
fn configured_resource_files_do_not_block_source_compilation() {
    let fixture = app("export const value = 1;");
    fixture.write("static/preference.json", b"{}");
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    project
        .extra_files
        .push(nativeboat_project::builder::ExtraFile {
            source: fixture.path("static/preference.json"),
            destination: "resources/static/preference.json".into(),
        });

    assert!(compile(&project).errors.is_empty());
}

#[test]
fn generated_application_runs_preloads_in_separate_ipc_connected_realms() {
    let fixture = app("import { app } from 'electron'; app.whenReady();");
    fixture.write(
        "index.html",
        b"<html><script type=module src='renderer.ts'></script></html>",
    );
    fixture.write("renderer.ts", b"console.log('renderer-ready');");
    fixture.write(
        "preload.ts",
        b"import { contextBridge, ipcRenderer, webFrame, webUtils } from 'electron'; contextBridge.exposeInMainWorld('api', { value: ipcRenderer.sendSync('ready'), path: webUtils.getPathForFile({ path: 'C:/note.md' }) }); webFrame.setZoomFactor(1.25);",
    );
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    project.preloads.push(nativeboat_project::Entry {
        source: fixture.path("preload.ts"),
        runtime_paths: vec!["out/preload/index.js".into()],
    });
    project.pages.push(nativeboat_project::Entry {
        source: fixture.path("index.html"),
        runtime_paths: vec!["index.html".into()],
    });
    let compilation = compile(&project);
    assert_eq!(compilation.preloads.len(), 1, "{:?}", compilation.errors);
    assert!(compilation.errors.is_empty(), "{:?}", compilation.errors);
    let generated = application_source(&project, &compilation, false, false).unwrap();
    assert!(generated.contains("preload_0::initialize"));
    assert!(generated.contains("preload_realm.connect_electron_ipc"));
    assert!(generated.contains("requested.ends_with(\"preload.js\")"));
    assert!(generated.contains("requested.ends_with(\"out/preload/index.js\")"));
    assert!(generated.contains("::nativeboat_native::context_bridge::expose("));
}

#[test]
fn generated_executable_runs_preload_and_sync_ipc_without_an_engine() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-preload-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { BrowserWindow, ipcMain } from 'electron';\n\
             ipcMain.on('native:test', (event, value) => { event.returnValue = value + 1; });\n\
             const window = new BrowserWindow({ webPreferences: { preload: './preload.js' } });\n\
             window.destroy();",
        ),
        (
            "preload.ts",
            "import { contextBridge, ipcRenderer, webFrame, webUtils } from 'electron';\n\
             const result = ipcRenderer.sendSync('native:test', 41);\n\
             if (result !== 42) throw new Error('synchronous preload IPC failed');\n\
             contextBridge.exposeInMainWorld('api', { result, path: webUtils.getPathForFile({ path: 'C:/note.md' }) });\n\
             webFrame.setZoomFactor(1.25);\n\
             console.log('preload-ready', result);",
        ),
    ]);
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executable = fixture.path("native-output/native-preload-test.exe");
    let executed = Command::new(executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "preload-ready 42\n"
    );
    assert!(executed.stderr.is_empty());
    assert!(
        imported_dlls(&fixture.path("native-output/native-preload-test.exe"))
            .iter()
            .all(|name| !name.contains("boa") && !name.contains("v8") && !name.contains("node"))
    );
}

#[test]
fn generated_renderers_read_their_load_url_and_report_uncompiled_pages() {
    // Pages load by URL with start-up arguments in the query, as MarkText's
    // windows do; the page path alone selects the compiled renderer.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-location-test","main":"main.cjs","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.cjs",
            "const { app, BrowserWindow } = require('electron');\nconst path = require('node:path');\nconst window = new BrowserWindow({ show: false });\nconst url = new URL(`file://${path.join(__dirname, 'index.html')}`);\nurl.searchParams.set('wid', '7');\nurl.searchParams.set('type', 'editor');\nwindow.loadURL(url.toString() + '#top').then(() => app.quit());",
        ),
        (
            "index.html",
            "<!doctype html><html><body><script type=module src='/renderer.js'></script></body></html>",
        ),
        (
            "renderer.js",
            "const params = new URLSearchParams(window.location.search); console.log(params.get('wid'), params.get('type'), location.protocol, location.hash, location.pathname.endsWith('/index.html'), document.location === location);",
        ),
    ]);
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executable = fixture.path("native-output/native-location-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "7 editor file: #top true true\n"
    );

    fixture.write(
        "main.cjs",
        b"const { BrowserWindow } = require('electron');\nnew BrowserWindow({ show: false }).loadURL('file:///C:/missing/other.html?x=1');",
    );
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executed = Command::new(&executable).output().unwrap();
    assert_eq!(executed.status.code(), Some(1), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stderr).unwrap(),
        "nativeboat: a window loaded 'file:///C:/missing/other.html?x=1', which this build did not compile\n"
    );
}

#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn generated_web_frames_run_callbacks_and_their_microtasks() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-frames-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, ipcMain } from 'electron'; ipcMain.on('done', () => app.quit()); const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.ts' } }); window.loadFile('index.html'); setTimeout(() => { console.log('frame timeout'); app.quit(); }, 2000);",
        ),
        (
            "preload.ts",
            "import { contextBridge, ipcRenderer } from 'electron'; contextBridge.exposeInMainWorld('nativeApi', { done: () => ipcRenderer.send('done') });",
        ),
        (
            "index.html",
            "<!doctype html><html><body><script type='module' src='./renderer.js'></script></body></html>",
        ),
        (
            "renderer.js",
            "const editor = document.createElement('main'); editor.style.cssText = 'width:120px;height:30px'; document.body.appendChild(editor); console.log('native geometry', editor.getBoundingClientRect().width, getComputedStyle(editor).height); requestAnimationFrame(time => { console.log('first frame', Number.isFinite(time)); Promise.resolve().then(() => requestAnimationFrame(next => { editor.style.width = '180px'; console.log('next frame', next > time, editor.getBoundingClientRect().width); window.nativeApi.done(); })); });",
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-frames-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(executed.stderr.is_empty(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "native geometry 120 30px\nfirst frame true\nnext frame true 180\n"
    );
}

#[test]
fn generated_executable_starts_a_renderer_graph_for_load_file() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-renderer-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow } from 'electron';\nconst window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.ts' } });\nwindow.loadFile('index.html').then(() => app.quit());",
        ),
        (
            "index.html",
            "<!doctype html><html><head></head><body><script type=module src='/renderer.ts'></script></body></html>",
        ),
        (
            "preload.ts",
            "import { contextBridge } from 'electron'; contextBridge.exposeInMainWorld('nativeApi', { value: 42 });",
        ),
        (
            "renderer.ts",
            "import './theme.css'; const original = document.createElement('span'); document.body.appendChild(original); const root = document.createElement('section'); root.id = 'runtime'; root.className = 'entry'; root.style.backgroundColor = 'rgb(1, 2, 3)'; root.style.setProperty('--editor-gap', '8px'); const text = document.createTextNode('ready'); root.appendChild(text); const marker = document.createComment('anchor'); root.insertBefore(marker, text); document.body.insertBefore(root, original); const found = document.getElementById('runtime'); found.textContent += '!'; const identity = document.querySelector('.entry') === found; const links = found.parentNode === document.body && found.nextSibling === original; const markerType = marker.nodeType === 8; const style = found.style.backgroundColor === 'rgb(1, 2, 3)' && found.style.getPropertyValue('--editor-gap') === '8px'; found.removeAttribute('class'); const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.setAttribute('viewBox', '0 0 2 2'); const svgCase = svg.namespaceURI.endsWith('svg') && svg.getAttribute('viewBox') === '0 0 2 2'; console.log('renderer-started', nativeApi.value, found.textContent, identity, links, markerType, found.hasAttribute('class'), style, svgCase); found.removeChild(marker);",
        ),
    ]);
    fixture.write("theme.css", b"body { margin: 0 } .entry { color: red }");
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executable = fixture.path("native-output/native-renderer-test.exe");
    let executed = Command::new(executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        executed.stdout,
        b"renderer-started 42 ready! true true true false true true\n"
    );
    assert!(executed.stderr.is_empty());
    assert!(
        imported_dlls(&fixture.path("native-output/native-renderer-test.exe"))
            .iter()
            .all(|name| !name.contains("boa") && !name.contains("v8") && !name.contains("node"))
    );
}

#[test]
fn pages_receive_messages_sent_on_did_finish_load_after_their_first_tasks() {
    // As MarkText's editor listens for its bootstrap message only after its
    // mount awaits.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-load-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, ipcMain } from 'electron';\n\
             const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.ts' } });\n\
             window.webContents.once('did-finish-load', () => window.webContents.send('boot', 'ready'));\n\
             ipcMain.on('booted', (_event, value) => { console.log('booted', value); app.quit(); });\n\
             setTimeout(() => { console.log('lost'); app.quit(); }, 3000);\n\
             window.loadFile('index.html').then(() => console.log('settled'));",
        ),
        (
            "index.html",
            "<!doctype html><html><body><script type=module src='/renderer.ts'></script></body></html>",
        ),
        (
            "preload.ts",
            "import { contextBridge, ipcRenderer } from 'electron';\n\
             contextBridge.exposeInMainWorld('api', {\n\
               on: (channel, listener) => ipcRenderer.on(channel, (_event, value) => listener(value)),\n\
               send: (channel, value) => ipcRenderer.send(channel, value),\n\
             });",
        ),
        (
            "renderer.ts",
            "(async () => {\n\
               await new Promise((resolve) => setTimeout(resolve, 0));\n\
               await Promise.resolve();\n\
               api.on('boot', (value) => api.send('booted', value));\n\
             })();",
        ),
    ]);
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executed = Command::new(fixture.path("native-output/native-load-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        // The load settles after the page loaded, before it answers.
        "settled\nbooted ready\n",
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}

#[test]
fn windows_opened_later_get_their_preloads_and_pages() {
    // As MarkText opens its preferences window from a menu.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-later-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, ipcMain } from 'electron';\n\
             const open = (name) => {\n\
               const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.ts' } });\n\
               window.webContents.once('did-finish-load', () => window.webContents.send('name', name));\n\
               return window.loadFile('index.html');\n\
             };\n\
             let seen = 0;\n\
             ipcMain.on('named', (_event, name) => {\n\
               console.log('page', name);\n\
               if (++seen === 1) setTimeout(() => open('later'), 10); else app.quit();\n\
             });\n\
             setTimeout(() => { console.log('lost'); app.quit(); }, 5000);\n\
             open('first');",
        ),
        (
            "index.html",
            "<!doctype html><html><body><script type=module src='/renderer.ts'></script></body></html>",
        ),
        (
            "preload.ts",
            "import { contextBridge, ipcRenderer } from 'electron';\n\
             contextBridge.exposeInMainWorld('api', {\n\
               on: (channel, listener) => ipcRenderer.on(channel, (_event, value) => listener(value)),\n\
               send: (channel, value) => ipcRenderer.send(channel, value),\n\
             });",
        ),
        (
            "renderer.ts",
            "api.on('name', (name) => api.send('named', name));",
        ),
    ]);
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executed = Command::new(fixture.path("native-output/native-later-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "page first\npage later\n",
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}

#[test]
fn page_scripts_end_in_checkpoints_that_settle_earlier_ipc() {
    // As ytDownloader's deferred i18n script awaits the locale from an
    // asynchronous main handler, and a later module script reads it.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-checkpoint-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, ipcMain } from 'electron';\n\
             ipcMain.handle('locale', async () => 'zh-CN');\n\
             ipcMain.on('seen', (_event, value) => { console.log('seen', value); app.quit(); });\n\
             setTimeout(() => { console.log('lost'); app.quit(); }, 5000);\n\
             new BrowserWindow({ show: false, webPreferences: { preload: './preload.ts' } }).loadFile('index.html');",
        ),
        (
            "index.html",
            "<!doctype html><html><head><script defer src='./first.js'></script><script type=module src='/second.ts'></script></head><body></body></html>",
        ),
        (
            "preload.ts",
            "import { contextBridge, ipcRenderer } from 'electron';\n\
             contextBridge.exposeInMainWorld('api', {\n\
               locale: () => ipcRenderer.invoke('locale'),\n\
               send: (channel, value) => ipcRenderer.send(channel, value),\n\
             });",
        ),
        (
            "first.js",
            "Promise.resolve().then(() => { window.order = 'microtask'; });\n\
             (async () => { window.saved = await api.locale(); })();",
        ),
        (
            "second.ts",
            "api.send('seen', `${window.order} ${window.saved}`);",
        ),
    ]);
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executed = Command::new(fixture.path("native-output/native-checkpoint-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "seen microtask zh-CN\n",
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}

#[test]
fn embedded_sources_include_every_native_runtime_module() {
    let runtime = include_str!("../../../nativeboat-native/src/lib.rs");
    let embedded: std::collections::HashSet<_> = super::SOURCES
        .iter()
        .map(|(name, _)| name.strip_suffix(".rs").unwrap())
        .collect();
    for line in runtime.lines() {
        let declaration = line
            .trim()
            .strip_prefix("pub mod ")
            .or_else(|| line.trim().strip_prefix("mod "));
        if let Some(module) = declaration.and_then(|name| name.strip_suffix(';')) {
            assert!(
                embedded.contains(module),
                "native module {module} is not embedded"
            );
        }
    }
}

#[test]
fn builds_native_async_delegation_with_node_reference_observations() {
    let source = format!(
        "{}\ncompleted.then(() => console.log(result));",
        include_str!("../../../nativeboat-compiler/src/dynamic/tests/async_delegation.mjs")
    );
    let fixture = app(&source);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!(
            "../../../nativeboat-compiler/src/dynamic/tests/async_delegation.expected.txt"
        )
        .replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_native_child_process_spawn_with_ordered_events_and_output() {
    let fixture = app("import { spawn } from 'node:child_process';\n\
         const child = spawn('where.exe', ['cmd.exe']);\n\
         const events = [];\n\
         child.on('spawn', () => events.push('spawn'));\n\
         child.stdout.setEncoding('utf8');\n\
         child.stdout.on('data', value => events.push(value.toString().trim()));\n\
         child.on('close', code => console.log(code, events.join(',')));");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let output = String::from_utf8(executed.stdout).unwrap();
    assert!(output.starts_with("0 spawn,"), "{output}");
    assert!(output.to_lowercase().contains("cmd.exe"), "{output}");
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_native_child_process_stdin_pipes() {
    let fixture = app("import { spawn } from 'node:child_process';\n\
         const child = spawn('findstr.exe', ['/c:needle']);\n\
         let output = '';\n\
         child.stdout.setEncoding('utf8');\n\
         child.stdout.on('data', value => output += value);\n\
         child.on('close', code => console.log(code, output.trim()));\n\
         child.stdin.write('needle from ');\n\
         child.stdin.end('stdin\\r\\n');");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"0 needle from stdin\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn builds_native_child_process_shell_spawn() {
    let fixture = app("import { spawn } from 'node:child_process';\n\
         const child = spawn('echo', ['native async shell'], { shell: true });\n\
         let output = '';\n\
         child.stdout.setEncoding('utf8');\n\
         child.stdout.on('data', value => output += value);\n\
         child.on('close', code => console.log(code, output.trim()));");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"0 native async shell\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn builds_native_child_process_exec_callbacks() {
    let fixture = app("import { exec, execFile } from 'node:child_process';\n\
         exec('echo native exec', (error, stdout) => console.log('exec', error === null, stdout.trim()));\n\
         execFile('where.exe', ['cmd.exe'], (error, stdout) => console.log('execFile', error === null, stdout.toLowerCase().includes('cmd.exe')));\n\
         execFile('where.exe', ['nativeboat-missing-command.exe'], (error, stdout, stderr) => console.log('failure', error.code, stdout.length, stderr.length));");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let output = String::from_utf8(executed.stdout).unwrap();
    let lines = output.lines().collect::<std::collections::HashSet<_>>();
    assert!(lines.contains("exec true native exec"), "{output}");
    assert!(lines.contains("execFile true true"), "{output}");
    let failure = lines
        .iter()
        .find(|line| line.starts_with("failure "))
        .expect("failed command invokes the error callback");
    let fields = failure.split_whitespace().collect::<Vec<_>>();
    assert_eq!(fields.len(), 4, "{output}");
    assert_eq!(fields[1..3], ["1", "0"], "{output}");
    assert!(fields[3].parse::<usize>().unwrap() > 0, "{output}");
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn streams_child_output_before_process_exit() {
    let fixture = app("import { spawn } from 'node:child_process';\n\
         const child = spawn('ping.exe', ['-n', '3', '127.0.0.1']);\n\
         let exited = false;\n\
         child.on('exit', () => exited = true);\n\
         child.stdout.once('data', () => console.log(exited ? 'buffered-until-exit' : 'streamed-before-exit'));\n\
         child.on('close', code => console.log('closed', code));");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"streamed-before-exit\nclosed 0\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn builds_inline_type_imports_without_loading_erased_modules() {
    let fixture = app(
        "import Store, { type Schema, count, next } from './store.ts';\n\
         import { type Missing } from 'electron';\n\
         import * as ns from './bridge.ts';\n\
         next(); const store: Schema = new Store();\n\
         console.log(store.value, count, ns.count, Object.keys(ns).join(','));",
    );
    fixture.write(
        "store.ts",
        b"export default class Store { value = 7; } export let count = 0; export function next() { count++; }",
    );
    fixture.write(
        "bridge.ts",
        b"export { type Schema, count } from './store.ts'; export { type Missing } from 'uncompiled-module';",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"7 1 1 count\n");
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    let prior = std::fs::read(&executable).unwrap();
    fixture.write(
        "main.ts",
        b"import { type Schema, absent } from './store.ts'; console.log(absent);",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("main.ts:1:23: module './store.ts' has no export 'absent'"),
        "{err}"
    );
    assert_eq!(std::fs::read(&executable).unwrap(), prior);
}

#[test]
fn builds_streamed_node_crypto_hashes_and_random_buffers_without_node() {
    let fixture = app("import crypto from 'crypto';\n\
         const sha1 = crypto.createHash('sha1').update('a').update('bc').digest('hex');\n\
         const sha256 = crypto.createHash('sha256').update('abc').digest('hex');\n\
         const random = crypto.randomBytes(24).toString('hex');\n\
         console.log(sha1, sha256, random.length, crypto.randomUUID().length);");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "a9993e364706816aba3e25717850c26c9cd0d89d ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad 48 36\n"
    );
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn compiles_and_runs_node_crypto() {
    // The expected text is Node 22.15's output for the same program: digests
    // and HMACs, AES modes, key derivation, random values, X.509 and errors.
    let fixture = app(include_str!("tests/node_crypto.mjs"));
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/node_crypto.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty(), "{:?}", executed.stderr);
}

#[test]
fn compiles_and_runs_web_crypto() {
    // The expected text is Node 22.15's output for the same program: SHA
    // digests, HMAC keys and signatures, AES-GCM, AES-CBC and AES-CTR,
    // PBKDF2 and HKDF, JWK export and import, and the errors of each.
    let fixture = app(include_str!("tests/web_crypto.mjs"));
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/web_crypto.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty(), "{:?}", executed.stderr);
}

/// A WebSocket echo server on the loopback interface for one connection:
/// it answers the opening handshake with the subprotocol the client offers
/// first, echoes text and binary messages, and answers a close frame with
/// the same status.
fn websocket_echo_server() -> (u16, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut log = Vec::new();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(20)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let (mut key, mut protocol) = (String::new(), None);
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                match name.trim().to_ascii_lowercase().as_str() {
                    "sec-websocket-key" => key = value.trim().to_owned(),
                    "sec-websocket-protocol" => {
                        protocol = value.split(',').next().map(|first| first.trim().to_owned());
                    }
                    _ => {}
                }
            }
        }
        let accept = base64(&sha1(
            format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
        ));
        let mut response = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n"
        );
        if let Some(protocol) = &protocol {
            response.push_str(&format!("Sec-WebSocket-Protocol: {protocol}\r\n"));
        }
        response.push_str("\r\n");
        writer.write_all(response.as_bytes()).unwrap();
        loop {
            let mut head = [0_u8; 2];
            if reader.read_exact(&mut head).is_err() {
                break;
            }
            let opcode = head[0] & 0x0f;
            let mut length = u64::from(head[1] & 0x7f);
            if length == 126 {
                let mut extended = [0_u8; 2];
                reader.read_exact(&mut extended).unwrap();
                length = u64::from(u16::from_be_bytes(extended));
            } else if length == 127 {
                let mut extended = [0_u8; 8];
                reader.read_exact(&mut extended).unwrap();
                length = u64::from_be_bytes(extended);
            }
            let mut mask = [0_u8; 4];
            reader.read_exact(&mut mask).unwrap();
            let mut payload = vec![0_u8; length as usize];
            reader.read_exact(&mut payload).unwrap();
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
            log.push(format!("{opcode}:{}", payload.len()));
            let mut frame = vec![0x80 | opcode];
            if payload.len() < 126 {
                frame.push(payload.len() as u8);
            } else {
                frame.push(126);
                frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            }
            frame.extend_from_slice(&payload);
            writer.write_all(&frame).unwrap();
            if opcode == 8 {
                break;
            }
        }
        log
    });
    (port, server)
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut words = [0_u32; 80];
        for (index, word) in block.chunks(4).enumerate() {
            words[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e]) {
            *value = value.wrapping_add(add);
        }
    }
    let mut digest = [0_u8; 20];
    for (index, value) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    digest
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let value = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                text.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// Node's `WebSocket` global: a client opens a connection with a
/// subprotocol, sends text, binary and Blob messages and receives their
/// echoes as strings, ArrayBuffers and Blobs, and closes cleanly with its
/// status and reason; constructor and `close` arguments fail as Node's do.
#[test]
fn compiles_and_runs_websocket_clients() {
    let (port, server) = websocket_echo_server();
    let fixture = app(&format!(
        r#"const out = [];
for (const [url, protocols] of [['ftp://x', undefined], ['ws://x/#a', undefined], ['ws://127.0.0.1:{port}', ['a', 'a']]]) {{
  try {{ new WebSocket(url, protocols); }} catch (error) {{ out.push(error.name); }}
}}
const socket = new WebSocket('ws://127.0.0.1:{port}/echo', ['chat', 'other']);
out.push(socket.readyState, socket.url, WebSocket.OPEN, socket.binaryType);
try {{ socket.send('early'); }} catch (error) {{ out.push(error.name); }}
let received = 0;
const messages = [];
socket.onopen = () => {{
  out.push('open', socket.readyState, socket.protocol);
  socket.send('hello');
  socket.send(new Uint8Array([1, 2, 3]));
  socket.send(new Blob(['blob']));
  try {{ socket.close(1001); }} catch (error) {{ out.push(error.name); }}
}};
socket.onmessage = (event) => {{
  const index = received++;
  const data = event.data;
  if (index === 1) socket.binaryType = 'arraybuffer';
  if (typeof data === 'string') messages[index] = `text ${{data}}`;
  else if (data instanceof ArrayBuffer) messages[index] = `buffer ${{Array.from(new Uint8Array(data)).join(',')}}`;
  else messages[index] = data.arrayBuffer().then((buffer) => `blob ${{Array.from(new Uint8Array(buffer)).join(',')}}`);
  if (index === 2) {{
    socket.close(4000, 'done');
    out.push(socket.readyState);
  }}
}};
socket.addEventListener('close', (event) => {{
  Promise.all(messages).then((list) => {{
    out.push(...list, 'close', event.wasClean, event.code, event.reason, socket.readyState);
    console.log(out.join('|'));
  }});
}});
socket.onerror = (event) => out.push('error', event.message);
"#
    ));
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        format!(
            "SyntaxError|SyntaxError|SyntaxError|0|ws://127.0.0.1:{port}/echo|1|blob|InvalidStateError|open|1|chat|InvalidAccessError|2|text hello|blob 1,2,3|buffer 98,108,111,98|close|true|4000|done|3\n"
        )
    );
    assert!(executed.stderr.is_empty(), "{:?}", executed.stderr);
    assert_eq!(server.join().unwrap(), ["1:5", "2:3", "2:4", "8:6"]);
}

/// An application's modules and its installed packages run from the
/// `resources/app` folder beside the executable, as a packaged
/// application's do: `import.meta` names that folder, and a package reads
/// the data file that ships there, not the build's copy.
#[test]
fn compiles_and_runs_modules_from_the_application_folder() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-test","type":"module","main":"main.mjs","dependencies":{"dep":"1"},"devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.mjs",
            "import { dirname, join } from 'node:path';\nimport { createRequire } from 'node:module';\nconst require = createRequire(import.meta.url);\nconst data = require('dep');\nconst app = join(dirname(process.execPath), 'resources', 'app');\nconsole.log([data, import.meta.dirname === app, import.meta.filename === join(app, 'main.mjs'), import.meta.url.startsWith('file:///') && import.meta.url.endsWith('/resources/app/main.mjs')].join('|'));\n",
        ),
        (
            "node_modules/dep/package.json",
            r#"{"name":"dep","main":"index.js"}"#,
        ),
        (
            "node_modules/dep/index.js",
            "module.exports = require('fs').readFileSync(require('path').join(__dirname, 'data.txt'), 'utf8').trim();",
        ),
        ("node_modules/dep/data.txt", "shipped data"),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        fixture
            .path("native-output/resources/app/node_modules/dep/data.txt")
            .is_file()
    );
    // The build's copy is gone: the program reads the shipped one.
    std::fs::remove_file(fixture.path("node_modules/dep/data.txt")).unwrap();
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "shipped data|true|true|true\n"
    );
}

/// Node's `navigator` global outside pages: the processor count, Node's
/// user agent and platform name, and the default locale.
#[test]
fn compiles_and_runs_node_navigator() {
    let fixture = app(
        "console.log([navigator.platform, typeof navigator.hardwareConcurrency, navigator.hardwareConcurrency > 0, navigator.userAgent === `Node.js/${process.versions.node.split('.')[0]}`, navigator.languages[0] === navigator.language, Object.prototype.toString.call(globalThis.navigator)].join('|'));\n",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "Win32|number|true|true|true|[object Navigator]\n"
    );
}

/// Functions whose text the program reads, by `f.toString()`, `${f}`,
/// `String(f)` or `'' + f`, keep their source text without TypeScript's
/// types, as programs that run a function's text elsewhere need. Other
/// functions read as functions without source text do.
#[test]
fn compiles_and_runs_functions_whose_source_text_is_read() {
    let fixture = app(r#"function fib(this: unknown, n: number): number;
function fib(this: unknown, n: number): number {
  return n <= 2 ? n : fib(n - 1) + fib(n - 2);
}
const square = (x: number): number => x * x;
const named = function twice<T>(value: T): T[] { return [value, value]; };
function plain(a: number) { return a; }
const flat = (text: string) => text.replace(/\s+/g, ' ').trim();
const unread = [plain][0];
function outer(prefix: string) {
  function inner(x: number) { return prefix + x; }
  return String(inner);
}
console.log([
  flat(outer('p')),
  flat(fib.toString()),
  flat(`${square}`),
  flat(String(named)),
  flat('' + (async (y: number) => { await null; return y as number; })),
  Function.prototype.toString.call(unread),
  Math.max.toString(),
  ((() => 1).bind(null) as Function).toString(),
  Object.prototype.toString.call(fib),
  String(fib(10)),
].join('\n'));
"#);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "function inner(x) { return prefix + x; }\n\
         function fib(n) { return n <= 2 ? n : fib(n - 1) + fib(n - 2); }\n\
         (x) => x * x\n\
         function twice(value) { return [value, value]; }\n\
         async (y) => { await null; return y; }\n\
         function plain() { [native code] }\n\
         function max() { [native code] }\n\
         function () { [native code] }\n\
         [object Function]\n\
         89\n"
    );
}

#[test]
fn compiles_and_runs_plugin_packages_loaded_by_path() {
    // A plugin package whose own build has not run: its main module is
    // compiled from the source its tsconfig.json emits it from, loads only
    // when the program requires its path, and gets the API the program
    // serves through Module._load, with its emitted path as parent.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-test","type":"module","main":"main.ts","nativeboat":{"plugins":["plugins/*"]},"devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            r#"import { createRequire } from 'module';
import Module from 'module';
import path from 'path';
import { realpathSync } from 'fs';
const require = createRequire(import.meta.url);
const dir = path.join(path.dirname(process.execPath), 'resources', 'app');
const original = (Module as any)._load;
(Module as any)._load = function (request: string, parent: any) {
  if (request === 'host-api') {
    return { from: path.relative(dir, realpathSync(parent.filename)).replace(/\\/g, '/') };
  }
  return original.apply(this, arguments);
};
console.log('main');
const plugin = require(path.join(dir, 'plugins', 'a', 'out', 'main.js'));
console.log(plugin.activate());
try {
  require(path.join(dir, 'plugins', 'gone.js'));
} catch (error: any) {
  console.log(error.code);
}
"#,
        ),
        (
            "plugins/a/package.json",
            r#"{"name":"a","main":"./out/main"}"#,
        ),
        (
            "plugins/a/tsconfig.json",
            r#"{"compilerOptions":{"rootDir":"./src","outDir":"./out"}}"#,
        ),
        (
            "plugins/a/src/main.ts",
            "import * as api from 'host-api';\n\
             import { helper } from './helper';\n\
             import installed from 'hostdep';\n\
             console.log('plugin loaded');\n\
             export function activate() { return [api.from, helper(), __dirname.endsWith('out'), installed].join(' '); }\n",
        ),
        // An installed package of the plugin that loads what the program
        // serves, as a telemetry package loads an editor's API.
        (
            "plugins/a/node_modules/hostdep/package.json",
            r#"{"name":"hostdep","main":"index.js"}"#,
        ),
        (
            "plugins/a/node_modules/hostdep/index.js",
            "module.exports = require('host-api').from;",
        ),
        (
            "plugins/a/src/helper.ts",
            "export function helper() { return 'helper'; }\n",
        ),
        ("plugins/a/src/data.txt", "beside the module"),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "main\nplugin loaded\nplugins/a/out/main.js helper true plugins/a/node_modules/hostdep/index.js\nMODULE_NOT_FOUND\n"
    );
    assert!(executed.stderr.is_empty(), "{:?}", executed.stderr);
    // Modules have files at their emitted paths; other files go beside them.
    for file in ["main.js", "helper.js", "data.txt"] {
        assert!(
            fixture
                .path("native-output/resources/app/plugins/a/out")
                .join(file)
                .is_file(),
            "{file}"
        );
    }
}

#[test]
fn builds_abort_controller_timeout_signal_without_node() {
    let fixture = app("const controller = new AbortController();\n\
         const signal = controller.signal;\n\
         signal.addEventListener('abort', () => console.log(signal.aborted, signal.reason));\n\
         controller.abort('timeout');\n\
         controller.abort('ignored');\n\
         console.log(signal.aborted, signal.reason);");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"true timeout\ntrue timeout\n");
    assert!(executed.stderr.is_empty());
    assert!(
        imported_dlls(&executable).iter().all(|name| {
            [
                "bcrypt.dll",
                "bcryptprimitives.dll",
                "kernel32.dll",
                "user32.dll",
                "winhttp.dll",
                "ws2_32.dll",
            ]
            .contains(&name.as_str())
        }),
        "unexpected DLL imports: {:?}",
        imported_dlls(&executable)
    );
}

#[test]
fn builds_vite_environment_and_static_defines_into_native_behavior() {
    let fixture = app(
        "if (import.meta.env.DEV !== false || MARKTEXT_VERSION !== '0.20.0' || MARKTEXT_VERSION_STRING !== 'v0.20.0') throw new Error('wrong build values'); console.log('build values ok');",
    );
    fixture.write(
        "electron.vite.config.ts",
        b"export default { main: { define: { MARKTEXT_VERSION: JSON.stringify('0.20.0'), MARKTEXT_VERSION_STRING: JSON.stringify('v0.20.0') } } };",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"build values ok\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn builds_vite_raw_assets_as_native_strings_and_reports_missing_sources() {
    let fixture = app("import template from './notification.html?raw';\n\
         console.log(template.includes('<p>Notice</p>'), template.includes('\\n'));");
    fixture.write("notification.html", b"<section>\n<p>Notice</p>\n</section>");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    std::fs::remove_file(fixture.path("notification.html")).unwrap();
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"true true\n");
    assert!(executed.stderr.is_empty());

    let previous = std::fs::read(&executable).unwrap();
    fixture.write(
        "main.ts",
        b"import template from './missing.html?raw'; console.log(template);",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("cannot resolve module './missing.html?raw'"),
        "{err}"
    );
    assert_eq!(std::fs::read(&executable).unwrap(), previous);

    fixture.write(
        "main.ts",
        b"import value from 'path?raw'; console.log(value);",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(err.contains("Vite asset import 'path?raw' must resolve to a local file"));
    assert_eq!(std::fs::read(&executable).unwrap(), previous);
}

#[test]
fn embeds_image_assets_and_vite_url_imports_in_native_modules() {
    let fixture = app(
        "import pngUrl from './icon.png'; import svgUrl from './icon.svg?url';\n\
         import avif from './image.avif'; import bmp from './image.bmp';\n\
         import gif from './image.gif'; import ico from './image.ico';\n\
         import jpeg from './image.jpg'; import webp from './image.webp';\n\
         if (pngUrl !== 'data:image/png;base64,iVBORw==') throw new Error(pngUrl);\n\
         if (svgUrl !== 'data:image/svg+xml;base64,PHN2Zy8+') throw new Error(svgUrl);\n\
         if (avif !== 'data:image/avif;base64,AA==' || bmp !== 'data:image/bmp;base64,AA==') throw new Error('wrong image mime');\n\
         if (gif !== 'data:image/gif;base64,AA==' || ico !== 'data:image/x-icon;base64,AA==') throw new Error('wrong image mime');\n\
         if (jpeg !== 'data:image/jpeg;base64,AA==' || webp !== 'data:image/webp;base64,AA==') throw new Error('wrong image mime');\n\
         console.log('embedded assets ok');",
    );
    fixture.write("icon.png", b"\x89PNG");
    fixture.write("icon.svg", b"<svg/>");
    for extension in ["avif", "bmp", "gif", "ico", "jpg", "webp"] {
        fixture.write(&format!("image.{extension}"), b"\0");
    }
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"embedded assets ok\n");
    assert!(executed.stderr.is_empty());
    assert!(!is_native_asset(None));
    assert!(!is_native_asset(Some("svg")));
    fixture.write("not-an-image.txt", b"text");
    fixture.write("extensionless", b"text");
    assert!(asset_module_source(&fixture.path("not-an-image.txt")).is_none());
    assert!(asset_module_source(&fixture.path("extensionless")).is_none());
    assert!(asset_module_source(&fixture.path("missing.png")).is_none());
}

#[test]
fn builds_vite_inline_css_as_native_strings() {
    let fixture = app(
        "import theme from './theme.css?inline'; if (theme !== 'body { color: red }\\n') throw new Error('inline CSS mismatch'); console.log(theme);",
    );
    fixture.write("theme.css", b"body { color: red }\n");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    std::fs::remove_file(fixture.path("theme.css")).unwrap();
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"body { color: red }\n\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn builds_vite_global_alias_as_the_native_global_object() {
    let fixture = app(
        "if (global !== globalThis || global.process !== process) throw new Error('global define mismatch'); console.log('global alias ok');",
    );
    fixture.write(
        "electron.vite.config.ts",
        b"export default { renderer: { define: { global: 'globalThis' } } };",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"global alias ok\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn project_compile_options_reject_dynamic_and_non_primitive_values() {
    let fixture = app("console.log('fixture');");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let environment = std::collections::BTreeMap::from([
        ("DEV".to_owned(), serde_json::Value::Bool(false)),
        (
            "MODE".to_owned(),
            serde_json::Value::String("production".to_owned()),
        ),
        ("COUNT".to_owned(), serde_json::json!(7)),
        ("EMPTY".to_owned(), serde_json::Value::Null),
        ("OBJECT".to_owned(), serde_json::json!({"dynamic": true})),
    ]);
    let defines = vec![
        ("VERSION".to_owned(), r#""0.20.0""#.to_owned()),
        ("global".to_owned(), "globalThis".to_owned()),
        ("DYNAMIC".to_owned(), "chooseVersion()".to_owned()),
        ("process.env.VERSION".to_owned(), "\"0.20.0\"".to_owned()),
        ("process.1env".to_owned(), "\"0.20.0\"".to_owned()),
        (String::new(), "\"bad key\"".to_owned()),
    ];
    let (options, errors) = compile_options(&project, &environment, &defines, true);
    assert_eq!(
        options.import_meta_env.as_ref().unwrap()["DEV"],
        CompileValue::Boolean(false)
    );
    assert_eq!(
        options.import_meta_env.as_ref().unwrap()["MODE"],
        CompileValue::String("production".to_owned())
    );
    assert_eq!(
        options.import_meta_env.as_ref().unwrap()["COUNT"],
        CompileValue::Number(7.0)
    );
    assert_eq!(
        options.import_meta_env.as_ref().unwrap()["EMPTY"],
        CompileValue::Null
    );
    assert!(
        !options
            .import_meta_env
            .as_ref()
            .unwrap()
            .contains_key("OBJECT")
    );
    assert_eq!(
        options.defines["VERSION"],
        CompileValue::String("0.20.0".to_owned())
    );
    assert_eq!(options.defines["global"], CompileValue::GlobalThis);
    // Dotted global paths are defines too, as Vite's `process.env.NODE_ENV`.
    assert_eq!(
        options.defines["process.env.VERSION"],
        CompileValue::String("0.20.0".to_owned())
    );
    assert_eq!(errors.len(), 4, "{errors:#?}");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("import.meta.env.OBJECT"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("define 'DYNAMIC'"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("define 'process.1env'"))
    );
    assert!(errors.iter().any(|error| error.contains("define ''")));
}

#[test]
fn builds_native_url_binding_reference_without_node() {
    build_node_helpers(
        include_str!("tests/url_binding.cjs"),
        include_str!("tests/url_binding.expected.txt"),
    );
}

#[test]
fn builds_native_filesystem_binding_against_node_reference() {
    let data = Fixture::new(&[]);
    let text = include_str!("tests/fs_binding.cjs")
        .replace("__ROOT__", &format!("{:?}", data.root().to_str().unwrap()))
        .replace("const root =", "require('internal/binding')('buffer').setBufferPrototype(Buffer.prototype);\nconst root =");
    build_node_helpers(&text, include_str!("tests/fs_binding.expected.txt"));
}

#[test]
fn builds_native_filesystem_capture_errors_with_node_delivery_order() {
    let data = Fixture::new(&[]);
    let text = include_str!("tests/fs_capture.cjs")
        .replace("__ROOT__", &format!("{:?}", data.root().to_str().unwrap()));
    build_node_helpers(&text, include_str!("tests/fs_capture.expected.txt"));
}

#[test]
fn builds_unchanged_node_directory_source_over_private_fs_dir_binding() {
    let fixture = Fixture::new(&[]);
    fixture.write("alpha.txt", b"a");
    fixture.write("child/beta.txt", b"b");
    let source = format!(
        "'use strict';\n\
         const {{ opendir, opendirSync }} = require('internal/fs/dir');\n\
         const dir = opendirSync({:?}, {{ bufferSize: 1 }});\n\
         let entry;\n\
         while ((entry = dir.readSync()) !== null) {{\n\
           console.log(entry.name, entry.isFile() ? 'file' : 'directory');\n\
         }}\n\
         dir.closeSync();\n\
         try {{ dir.readSync(); }} catch (error) {{ console.log(error.code); }}\n\
         opendir({:?}, {{ bufferSize: 1 }}, (error, asyncDir) => {{\n\
           if (error) throw error;\n\
           asyncDir.read((readError, asyncEntry) => {{\n\
             if (readError) throw readError;\n\
             console.log('async', asyncEntry.name, asyncEntry.isFile() ? 'file' : 'directory');\n\
             asyncDir.close((closeError) => {{\n\
               if (closeError) throw closeError;\n\
               console.log('async closed');\n\
             }});\n\
           }});\n\
         }});",
        fixture.root().to_str().unwrap(),
        fixture.root().to_str().unwrap()
    );
    build_node_helpers(
        &source,
        "alpha.txt file\nchild directory\nERR_DIR_CLOSED\nasync alpha.txt file\nasync closed\n",
    );
}

#[test]
fn builds_unchanged_streaming_decoder_with_node_split_point_observations() {
    build_node_helpers(
        include_str!("tests/string_decoder.cjs"),
        include_str!("tests/string_decoder.expected.txt"),
    );
}

#[test]
fn compiles_and_runs_public_string_decoder_without_node() {
    let fixture = app("import { StringDecoder } from 'string_decoder';\n\
         const decoder = new StringDecoder('utf8');\n\
         const first = decoder.write(Buffer.from([0xe2, 0x82]));\n\
         const state = `${decoder.lastNeed}:${decoder.lastTotal}:${Array.from(decoder.lastChar).join(',')}`;\n\
         const second = decoder.write(Buffer.from([0xac]));\n\
         console.log(`${first.length}:${state}:${Array.from(second, c => c.charCodeAt(0)).join(',')}:${decoder.end().length}`);");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"0:1:3:226,130,0,0:8364:0\n");
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn compiles_and_runs_public_filesystem_sources_without_node() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-public-fs","main":"main.cjs","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.cjs",
            "'use strict';\n\
             const fs = require('fs');\n\
             const { readFile } = require('fs/promises');\n\
             let polls = 0;\n\
             const pollingTimeout = setTimeout(() => { fs.unwatchFile('polling.txt'); throw new Error('filesystem polling timed out'); }, 3000);\n\
             fs.watchFile('polling.txt', { interval: 5 }, (current, previous) => {\n\
               if (polls++ === 0) { console.log(`poll missing ${current.size} ${previous.size}`); fs.writeFileSync('polling.txt', 'polling'); }\n\
               else { console.log(`poll create ${current.size} ${previous.size}`); fs.unwatchFile('polling.txt'); clearTimeout(pollingTimeout); }\n\
             });\n\
             const directory = __dirname;\n\
             const watcher = fs.watch(directory, { persistent: false });\n\
             const fileWatcher = fs.watch('WATCHED.TXT', { persistent: false }, (event, filename) => { console.log(`watch ${event} ${filename}`); fileWatcher.close(); });\n\
             fs.writeFileSync('watched.txt', 'watched changed');\n\
             const text = fs.readFileSync('fixture.txt', 'utf8');\n\
             console.log(text);\n\
             const descriptor = fs.openSync('fixture.txt', 'r');\n\
             console.log(fs.readFileSync(descriptor, 'utf8'));\n\
             fs.closeSync(descriptor);\n\
             readFile('fixture.txt', 'utf8').then((text) => console.log(`promise ${text}`));\n\
             console.log(typeof watcher.close);\n\
             watcher.close();",
        ),
        ("fixture.txt", "public filesystem"),
        ("watched.txt", "before"),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-public-fs.exe");
    let executed = Command::new(&executable)
        .current_dir(fixture.root())
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let mut output = String::from_utf8(executed.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    output.sort();
    let mut expected = [
        "public filesystem",
        "public filesystem",
        "function",
        "promise public filesystem",
        "watch change watched.txt",
        "poll missing 0 0",
        "poll create 7 0",
    ]
    .map(str::to_owned);
    expected.sort();
    assert_eq!(output, expected);
    assert!(executed.stderr.is_empty(), "{:?}", executed.stderr);
    let imports = imported_dlls(&executable);
    assert!(
        imports.iter().all(|name| {
            [
                "kernel32.dll",
                "bcryptprimitives.dll",
                "bcrypt.dll",
                "user32.dll",
                "winhttp.dll",
                "ws2_32.dll",
            ]
            .contains(&name.as_str())
        }),
        "unexpected PE imports: {imports:?}"
    );
}

#[test]
fn builds_nodes_internal_util_helpers_without_an_engine() {
    build_node_helpers(
        include_str!("tests/util_helpers.cjs"),
        include_str!("tests/util_helpers.expected.txt"),
    );
}

#[test]
fn builds_and_runs_public_util_format_without_an_engine() {
    build_node_helpers(
        "'use strict'; const util = require('util'); console.log(util.format('%s %d', 'value', 42));",
        "value 42\n",
    );
}

#[test]
fn builds_protected_regexp_helpers_without_a_vm() {
    build_node_helpers(
        include_str!("tests/protected_regexp.cjs"),
        include_str!("tests/protected_regexp.expected.txt"),
    );
}

#[test]
fn builds_url_transfer_marking_with_hidden_native_keys() {
    build_node_helpers(
        include_str!("tests/transfer_marking.cjs"),
        include_str!("tests/transfer_marking.expected.txt"),
    );
}

fn build_node_helpers(text: &str, expected: &str) {
    // Exercise private and public Node helpers through a virtual Node source graph.
    let fixture = app("console.log('fixture metadata only');");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let entry = Path::new("node:nativeboat/util-test.js");
    let module = compile_sources(
        entry,
        &mut |from, name, _| crate::node_lib::internal(from, name),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: text.into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                crate::node_lib::source(path)
            }
        },
    )
    .unwrap();
    let args = settings(&fixture);
    let compilation = NativeCompilation {
        main: Some(module),
        preloads: Vec::new(),
        forks: Vec::new(),
        renderers: Vec::new(),
        stylesheets: std::collections::BTreeMap::new(),
        errors: Vec::new(),
    };
    let executable = link(&project, &compilation, &output(&project, &args), &args).unwrap();
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        expected.replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty(), "{:?}", executed.stderr);
    let bytes = std::fs::read(&executable).unwrap();
    let notice = nativeboat_url_build::NOTICES.as_bytes();
    assert!(bytes.windows(notice.len()).any(|part| part == notice));
    // The same notices lie beside the executable, to ship with it.
    let file = std::fs::read_to_string(executable.with_file_name(NOTICES_FILE)).unwrap();
    assert_eq!(file, app_notices());
    assert!(file.contains("taffy") && file.contains(nativeboat_url_build::NOTICES));
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn library_backed_global_members_link_without_imports() {
    let fixture = app(
        "const C=globalThis.URL; const key='URLSearchParams'; const P=global[key]; const u=new C('/a?x=1','https://host/b'); console.log(C===URL,P===URLSearchParams,u.href,u.searchParams instanceof P); const local={URL:7}; function shadow(globalThis){return globalThis.URL;} console.log(shadow(local));",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "true true https://host/a?x=1 true\n7\n"
    );
    assert!(executed.stderr.is_empty());
}

#[test]
fn builds_node_url_source_and_shared_global_constructors() {
    let fixture = app(include_str!("tests/url.js"));
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/url.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty(), "{:?}", executed.stderr);
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_native_url_search_params_from_node_source() {
    let fixture = app(include_str!("tests/url_search_params.js"));
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/url_search_params.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty(), "{:?}", executed.stderr);
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_node_querystring_from_nodes_own_library_source() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-querystring","main":"main.cjs","devDependencies":{"electron":"42"}}"#,
        ),
        ("main.cjs", include_str!("tests/querystring.cjs")),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-querystring.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/querystring.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    fixture.write(
        "main.cjs",
        b"'use strict'; require('querystring'); require('electron');",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(Command::new(&executable).output().unwrap().status.success());
    fixture.write("package.json", br#"{"name":"native-querystring","type":"module","main":"main.mjs","devDependencies":{"electron":"42"}}"#);
    fixture.write("main.mjs", include_bytes!("tests/querystring.mjs"));
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(executed.stderr.is_empty());
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "true true true true {\"a\":[\"one two\",\"3\"]} a%20b\n"
    );
}

#[test]
fn builds_native_keyed_collections_and_console_inspection() {
    let fixture = app(
        "const map = new Map([['a',1],['b',2]]); const set = new Set('aba'); if(map.size!==2||set.size!==2||map.get('b')!==2)throw 'collections'; console.log(map,set); map.delete('a');map.set('a',3);if([...map.keys()].join(',')!=='b,a')throw 'order';",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        executed.stdout,
        b"Map(2) { 'a' => 1, 'b' => 2 } Set(2) { 'a', 'b' }\n"
    );
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    for name in ["Map", "Set"] {
        fixture.write("main.ts", format!("{name} = {{}};").as_bytes());
        let (code, _, err) = run(&fixture, &["--out", "native-output"]);
        assert_eq!(code, 0, "{err}");
        let replaced = Command::new(&executable).output().unwrap();
        assert!(!replaced.status.success(), "{replaced:?}");
        assert!(
            String::from_utf8_lossy(&replaced.stderr).contains(&format!(
                "TypeError: Replacing the global {name} is not supported natively"
            )),
            "{replaced:?}"
        );
    }
}

fn app(source: &str) -> Fixture {
    Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        ("main.ts", source),
    ])
}

/// Runs a built program to its end, killing it and failing the test with
/// what it printed so far when it runs longer than `limit`, so a program
/// that never exits cannot hang the test run.
fn output_within(command: &mut Command, limit: std::time::Duration) -> std::process::Output {
    use std::io::Read;
    use std::sync::{Arc, Mutex};
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let collect = |mut stream: Box<dyn Read + Send>| {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let sink = bytes.clone();
        std::thread::spawn(move || {
            let mut chunk = [0; 4096];
            while let Ok(count @ 1..) = stream.read(&mut chunk) {
                sink.lock().unwrap().extend_from_slice(&chunk[..count]);
            }
        });
        bytes
    };
    let stdout = collect(Box::new(child.stdout.take().unwrap()));
    let stderr = collect(Box::new(child.stderr.take().unwrap()));
    let text =
        |bytes: &Arc<Mutex<Vec<u8>>>| String::from_utf8_lossy(&bytes.lock().unwrap()).into_owned();
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "the program ran longer than {limit:?}; stdout: {}; stderr: {}",
                text(&stdout),
                text(&stderr)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    // The readers end with the program's pipes; give them a moment to drain.
    std::thread::sleep(std::time::Duration::from_millis(200));
    std::process::Output {
        status,
        stdout: stdout.lock().unwrap().clone(),
        stderr: stderr.lock().unwrap().clone(),
    }
}

fn run(fixture: &Fixture, flags: &[&str]) -> (u8, String, String) {
    let mut args = vec![
        "nativeboat".to_owned(),
        "build".to_owned(),
        fixture.root().to_string_lossy().into_owned(),
    ];
    args.extend(flags.iter().map(|flag| flag.to_string()));
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::run(args, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

/// The DLLs an executable imports, read from its PE import directory.
fn imported_dlls(executable: &Path) -> Vec<String> {
    let data = std::fs::read(executable).unwrap();
    let u16_at = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    let header = u32_at(0x3c);
    let sections = u16_at(header + 6);
    let optional = header + 24;
    let table = optional + u16_at(header + 20);
    // The import directory is data directory 1 of a PE32+ optional header.
    let imports = u32_at(optional + 112 + 8);
    let offset = |rva: usize| {
        (0..sections)
            .map(|index| table + index * 40)
            .find_map(|section| {
                let (size, address, raw) = (
                    u32_at(section + 8),
                    u32_at(section + 12),
                    u32_at(section + 20),
                );
                (address..address + size)
                    .contains(&rva)
                    .then(|| rva - address + raw)
            })
            .unwrap()
    };
    let mut names = Vec::new();
    let mut entry = offset(imports);
    while u32_at(entry + 12) != 0 {
        let start = offset(u32_at(entry + 12));
        let end = start + data[start..].iter().position(|byte| *byte == 0).unwrap();
        names.push(String::from_utf8_lossy(&data[start..end]).to_ascii_lowercase());
        entry += 20;
    }
    names
}

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

fn success() -> Output {
    Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

#[test]
fn native_command_defaults_to_source_project_and_toolchain() {
    use clap::Parser;
    let cli = crate::Cli::try_parse_from(["nativeboat", "build"]).unwrap();
    let crate::Command::Build(args) = cli.command else {
        panic!("expected build")
    };
    assert_eq!(args.project, Path::new("."));
    assert!(args.out.is_none());
    assert!(args.rustc.is_none());
}

#[test]
fn builds_and_executes_source_graph_with_aliases_types_and_import_conditions() {
    let fixture = app(
        "import {next} from '@/counter.js'; import {offset} from 'choice'; if (next() !== 2 || next() !== 3 || offset !== 7) throw 'wrong native graph'; function again() { return again(); } let limited = false; try { again(); } catch (error) { limited = error.name === 'RangeError'; } if (!limited) throw 'wrong native call limit';",
    );
    fixture.write(
        "tsconfig.json",
        br#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    );
    fixture.write(
        "src/counter.ts",
        b"let value: number = 1; export function next() { return ++value; }",
    );
    fixture.write(
        "node_modules/choice/package.json",
        br#"{"exports":{"require":"./cjs.js","import":"./esm.js"}}"#,
    );
    fixture.write("node_modules/choice/cjs.js", b"throw 'wrong condition';");
    fixture.write("node_modules/choice/esm.js", b"export const offset = 7;");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    assert!(err.is_empty());
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{:?}", executed);
    assert!(
        nativeboat_ir::package::read_executable(&executable)
            .unwrap()
            .is_none()
    );
    // Linked without MSVC, a C runtime or Windows SDK libraries: only
    // operating-system DLLs are imported.
    let mut dlls = imported_dlls(&executable);
    dlls.sort();
    assert_eq!(
        dlls,
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll"
        ]
    );
    // Every executable carries the notices of the third-party code it contains.
    let image = std::fs::read(&executable).unwrap();
    let notice = b"Developed at SunPro, a Sun Microsystems, Inc. business.";
    assert!(image.windows(notice.len()).any(|window| window == notice));
    // The output holds the executable and the notices to ship with it.
    let mut outputs: Vec<_> = std::fs::read_dir(fixture.path("native-output"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    outputs.sort();
    // and the application's own files in resources/app, without the
    // build's configuration.
    assert_eq!(
        outputs,
        ["THIRD-PARTY-NOTICES.txt", "native-test.exe", "resources"]
    );
    assert!(
        fixture
            .path("native-output/resources/app/package.json")
            .is_file()
    );
    assert!(!fixture.path("dist").exists());
    // Electron graphs link the host message loop and readiness exports.
    fixture.write("main.ts", b"import {app} from 'electron'; app.whenReady();");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(Command::new(&executable).output().unwrap().status.success());
    assert!(imported_dlls(&executable).contains(&"user32.dll".to_owned()));
    // Successful publication replaces it, and uncaught native throws fail.
    fixture.write("main.ts", b"throw 'native failure';");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(&executable).output().unwrap();
    assert_eq!(executed.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(executed.stderr).unwrap(),
        "Uncaught native failure
"
    );
}

#[test]
fn builds_native_error_constructors_and_preserves_output_on_failed_builds() {
    let fixture = app(
        "const error = new TypeError('bad', {cause:7}); const aggregate = AggregateError([error]); if (!(error instanceof Error) || !Error.isError(error) || error.cause !== 7 || aggregate.errors[0] !== error || aggregate.toString() !== 'AggregateError') throw 'wrong native errors';",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let prior = std::fs::read(&executable).unwrap();
    // A build that fails leaves the previous executable in place.
    fixture.write("main.ts", b"const = 1;");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(err.contains("main.ts:1:"), "{err}");
    assert_eq!(std::fs::read(&executable).unwrap(), prior);
}

#[test]
fn builds_native_json_and_console_without_engines_or_external_libraries() {
    let fixture = app(
        "const parsed = JSON.parse('{\"a\":1}'); console.log('%s %j', 'native', parsed); console.error('stderr %d', parsed.a); console.time('clock'); console.timeEnd('clock');",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let stdout = String::from_utf8(executed.stdout).unwrap();
    assert!(stdout.starts_with("native {\"a\":1}\nclock: "), "{stdout}");
    assert_eq!(executed.stderr, b"stderr 1\n");
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    for name in ["JSON", "console"] {
        fixture.write("main.ts", format!("{name} = {{}};").as_bytes());
        let (code, _, err) = run(&fixture, &["--out", "native-output"]);
        assert_eq!(code, 0, "{err}");
        let replaced = Command::new(&executable).output().unwrap();
        assert!(!replaced.status.success(), "{replaced:?}");
        assert!(
            String::from_utf8_lossy(&replaced.stderr).contains(&format!(
                "TypeError: Replacing the global {name} is not supported natively"
            )),
            "{replaced:?}"
        );
    }
}

#[test]
fn builds_native_regular_expressions_with_the_packaged_matcher() {
    let fixture = app(
        r"const date = /(?<year>\d{4})-(\d{2})/u.exec('on 2026-10'); console.log(date.groups.year, 'a1b2'.replace(/\d/g, '#'), 'x, y'.split(/,\s*/).length);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(String::from_utf8(executed.stdout).unwrap(), "2026 a#b# 2\n");
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    // The matcher's notices, including the Unicode data license, ship too.
    let bytes = std::fs::read(&executable).unwrap();
    let notice = nativeboat_regexp_build::NOTICES.as_bytes();
    assert!(bytes.windows(notice.len()).any(|part| part == notice));
    fixture.write("main.ts", b"export const pattern = /(?<a>x)(?<a>y)/;");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("invalid regular expression: Duplicate capture group name"),
        "{err}"
    );
    assert_eq!(std::fs::read(&executable).unwrap(), bytes);
    // Expressions that are not compiled yet remain diagnosed by kind.
    fixture.write("main.ts", b"export const url = import.meta.resolve;");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(err.contains("main.ts:1:20: "), "{err}");
    assert!(err.contains(" is not compiled yet"), "{err}");
    assert_eq!(std::fs::read(&executable).unwrap(), bytes);
}

#[test]
fn native_libraries_are_cached_per_toolchain() {
    let fixture = Fixture::new(&[("blocked", "a file where the cache would go")]);
    let rustc = Toolchain {
        rustc: PathBuf::from("rustc"),
        sysroot: None,
    };
    let error = toolchain_key(&rustc, &fixture.path("blocked/cache")).unwrap_err();
    assert!(error.starts_with("cannot create cache"), "{error}");
    let cache = fixture.path("cache");
    let key = toolchain_key(&rustc, &cache).unwrap();
    let (built, copy) = matcher(&rustc, &cache, key).unwrap();
    assert!(copy.is_none());
    let (url, copy) = url_parser(&rustc, &cache, key).unwrap();
    assert!(copy.is_none() && url.join("liburl.rlib").is_file());
    let (reused, copy) = url_parser(&rustc, &cache, key).unwrap();
    assert_eq!((reused, copy.is_none()), (url.clone(), true));
    let (encoding, copy) = encoding_detector(&rustc, &cache, key, &built).unwrap();
    assert!(copy.is_none() && encoding.join("libchardetng.rlib").is_file());
    let library = built.join("libnativeboat_regexp.rlib");
    assert!(library.is_file());
    // Older runtime builds beyond the newest few are removed on publishing.
    for index in 0..CACHE_KEEP {
        std::fs::create_dir(cache.join(format!("runtime-old{index}"))).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (runtime_built, copy) =
        runtime(&rustc, &cache, key, (&built, &url, &encoding), None).unwrap();
    assert!(!cache.join("runtime-old0").exists());
    assert!(cache.join("runtime-old1").exists() && cache.join("runtime-old2").exists());
    assert!(copy.is_none());
    assert!(runtime_built.join("libnativeboat_start.rlib").is_file());
    // Later builds reuse the published libraries.
    let (reused, copy) = matcher(&rustc, &cache, key).unwrap();
    assert_eq!((reused, copy.is_none()), (built.clone(), true));
    let (reused, copy) = runtime(&rustc, &cache, key, (&built, &url, &encoding), None).unwrap();
    assert_eq!((reused, copy.is_none()), (runtime_built.clone(), true));
    // The Node.js library is built against that runtime and reused too.
    let (node, copy) = node_library(
        &rustc,
        &cache,
        &runtime_built,
        (&built, &url, &encoding),
        None,
        &["events".to_owned()].into_iter().collect(),
    )
    .unwrap();
    assert!(copy.is_none() && node.join("libnativeboat_node.rlib").is_file());
    let (reused, copy) = node_library(
        &rustc,
        &cache,
        &runtime_built,
        (&built, &url, &encoding),
        None,
        &["events".to_owned()].into_iter().collect(),
    )
    .unwrap();
    assert_eq!((reused, copy.is_none()), (node.clone(), true));
    let generated_source = "#![no_std]\nextern crate alloc;\nextern crate nativeboat_native;\npub fn cached_value() -> usize { 7 }\n";
    let generated_name = "nativeboat_test_generated";
    let generated_fingerprint =
        generated_rlib_fingerprint(key, generated_name, generated_source, &[]);
    let generated_staging = fixture.path("generated-staging");
    std::fs::create_dir(&generated_staging).unwrap();
    let generated_dependencies: [&Path; 4] = [&runtime_built, &built, &url, &encoding];
    let (generated, cache_hit) = compile_generated_rlib_cached(
        &rustc,
        &cache,
        &generated_staging,
        &generated_dependencies,
        GeneratedRlib {
            externs: &[],
            crate_name: generated_name,
            source_name: "generated.rs",
            source: generated_source,
            fingerprint: generated_fingerprint,
            application: true,
        },
    )
    .unwrap();
    assert!(!cache_hit && generated.is_file());
    let (reused_generated, cache_hit) = compile_generated_rlib_cached(
        &rustc,
        &cache,
        &generated_staging,
        &generated_dependencies,
        GeneratedRlib {
            externs: &[],
            crate_name: generated_name,
            source_name: "generated.rs",
            source: generated_source,
            fingerprint: generated_fingerprint,
            application: true,
        },
    )
    .unwrap();
    assert!(cache_hit);
    assert_eq!(reused_generated, generated);
    // Rebuilt in another staging folder, the same sources compile to the
    // same bytes, so the keys of the libraries depending on them hold.
    let first = std::fs::read(&generated).unwrap();
    std::fs::remove_dir_all(generated.parent().unwrap()).unwrap();
    let (rebuilt, cache_hit) = compile_generated_rlib_cached(
        &rustc,
        &cache,
        &generated_staging,
        &generated_dependencies,
        GeneratedRlib {
            externs: &[],
            crate_name: generated_name,
            source_name: "generated.rs",
            source: generated_source,
            fingerprint: generated_fingerprint,
            application: true,
        },
    )
    .unwrap();
    assert!(!cache_hit);
    assert!(
        std::fs::read(&rebuilt).unwrap() == first,
        "generated library bytes differ"
    );
    let node_bytes = std::fs::read(node.join("libnativeboat_node.rlib")).unwrap();
    std::fs::remove_dir_all(&node).unwrap();
    let (rebuilt_node, _copy) = node_library(
        &rustc,
        &cache,
        &runtime_built,
        (&built, &url, &encoding),
        None,
        &["events".to_owned()].into_iter().collect(),
    )
    .unwrap();
    assert!(
        std::fs::read(rebuilt_node.join("libnativeboat_node.rlib")).unwrap() == node_bytes,
        "node library bytes differ"
    );
    // A build that cannot be published uses its own copy, removed after use.
    std::fs::remove_file(&library).unwrap();
    let (own, copy) = matcher(&rustc, &cache, key).unwrap();
    assert_ne!(own, built);
    assert!(own.join("libnativeboat_regexp.rlib").is_file());
    drop(copy);
    assert!(!own.exists());
    // A toolchain's sysroot is part of its identity.
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    let sysroot = PathBuf::from(String::from_utf8(sysroot.stdout).unwrap().trim());
    let rooted = Toolchain {
        rustc: PathBuf::from("rustc"),
        sysroot: Some(sysroot),
    };
    assert_ne!(toolchain_key(&rooted, &cache).unwrap(), key);
    let error = write_sources(&fixture.path("blocked"), &[("lib.rs", "")]).unwrap_err();
    assert!(error.starts_with("cannot write"), "{error}");
}

#[test]
fn packaged_renderer_libraries_are_content_cached_and_require_layout() {
    let fixture = Fixture::new(&[]);
    let source = fixture.path("toolchain/renderer-libs");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("libnativeboat_layout-abc.rlib"), b"layout").unwrap();
    std::fs::write(source.join("libnativeboat_dom-def.rlib"), b"dom").unwrap();
    std::fs::write(source.join("serde_derive-ghi.dll"), b"proc-macro").unwrap();
    let tool = Toolchain {
        rustc: PathBuf::from("rustc"),
        sysroot: Some(fixture.path("toolchain")),
    };
    let cache = fixture.path("cache");
    let (first, copy) = renderer_libraries(&tool, &cache, 7).unwrap().unwrap();
    assert!(copy.is_none());
    assert_eq!(
        std::fs::read(first.join("libnativeboat_dom-def.rlib")).unwrap(),
        b"dom"
    );
    assert!(first.join("serde_derive-ghi.dll").is_file());
    let (reused, copy) = renderer_libraries(&tool, &cache, 7).unwrap().unwrap();
    assert_eq!((reused, copy.is_none()), (first.clone(), true));

    std::fs::write(source.join("libnativeboat_dom-def.rlib"), b"updated dom").unwrap();
    let (updated, copy) = renderer_libraries(&tool, &cache, 7).unwrap().unwrap();
    assert!(copy.is_none());
    assert_ne!(updated, first);

    std::fs::remove_file(source.join("libnativeboat_layout-abc.rlib")).unwrap();
    std::fs::write(source.join("libnativeboat_css-ghi.rlib"), b"css").unwrap();
    assert!(renderer_libraries(&tool, &cache, 7).is_err());
    assert!(
        renderer_libraries(
            &Toolchain {
                rustc: PathBuf::from("rustc"),
                sysroot: None,
            },
            &cache,
            7
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn cache_builds_that_lose_the_publish_race_use_the_published_copy() {
    let fixture = Fixture::new(&[]);
    let cache = fixture.path("cache");
    std::fs::create_dir(&cache).unwrap();
    let published = cache.join("runtime-test");
    let (directory, guard) = cached(&cache, "runtime-test", "runtime.rlib", |staging| {
        // Another build publishes while this one is still building.
        std::fs::create_dir_all(&published).unwrap();
        std::fs::write(published.join("runtime.rlib"), b"first").unwrap();
        std::fs::create_dir_all(staging).unwrap();
        std::fs::write(staging.join("runtime.rlib"), b"second").unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(directory, published);
    assert!(guard.is_none());
    assert_eq!(
        std::fs::read(directory.join("runtime.rlib")).unwrap(),
        b"first"
    );
    // This build's own copy is removed.
    let entries: Vec<_> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, ["runtime-test"]);
}

#[test]
fn failed_cache_build_removes_its_staging_directory() {
    let fixture = Fixture::new(&[]);
    let cache = fixture.path("cache");
    std::fs::create_dir(&cache).unwrap();
    let result = cached(&cache, "runtime-test", "runtime.rlib", |_| {
        Err("fixture build failed".to_owned())
    });
    let error = match result {
        Ok(_) => panic!("failed fixture build unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error, "fixture build failed");
    assert_eq!(std::fs::read_dir(cache).unwrap().count(), 0);
}

#[test]
fn raw_module_source_returns_none_when_the_imported_file_disappears() {
    let fixture = Fixture::new(&[]);
    assert!(raw_module_source(&fixture.path("missing.bin")).is_none());
}

#[test]
fn invoke_resolves_a_tool_path_with_multiple_components() {
    let fixture = Fixture::new(&[]);
    let error = invoke(
        Path::new("missing/nativeboat-rustc.exe"),
        &[],
        fixture.root(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn builds_native_promises_and_timers_with_an_event_loop() {
    let fixture = app(
        "const wait = (ms, value) => new Promise((resolve) => setTimeout(resolve, ms, value)); wait(5, 'later').then((value) => console.log(value)); Promise.resolve('soon').then((value) => console.log(value)); setImmediate(() => console.log('immediate')); console.log('now');",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "now\nsoon\nimmediate\nlater\n"
    );
    // Unhandled rejections end the program with an error, as in Node.
    fixture.write(
        "main.ts",
        b"Promise.reject(new Error('lost')); setTimeout(() => console.log('never'), 1);",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let executed = Command::new(&executable).output().unwrap();
    assert_eq!(executed.status.code(), Some(1));
    assert!(executed.stdout.is_empty());
    assert_eq!(
        String::from_utf8(executed.stderr).unwrap(),
        "Uncaught Error: lost\n"
    );
}

#[test]
fn builds_native_generators_and_async_functions_on_fibers() {
    let fixture = app(
        "function* ids() { let id = 0; while (true) yield ++id; } const next = ids(); async function save(name) { await new Promise((resolve) => setTimeout(resolve, 2)); return name + '#' + next.next().value; } (async () => { const saved = await Promise.all([save('a'), save('b')]); console.log(saved.join(' ')); try { await Promise.reject(new Error('denied')); } catch (error) { console.log('caught ' + error.message); } })();",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "a#1 b#2\ncaught denied\n"
    );
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_native_bound_commonjs_constructors_and_array_like_calls() {
    let fixture = app(
        "import BoundPoint from './point.cjs'; const point = new BoundPoint(4); function sum(value) {return this + value;} const bound = sum.bind(5,6); if (point.x !== 3 || point.y !== 4 || !(point instanceof BoundPoint) || bound.apply(null,[]) !== 11 || sum.apply(7,{0:8,length:1}) !== 15) throw 'wrong native bound functions';",
    );
    fixture.write("point.cjs", b"'use strict'; function Point(x,y) {this.x=x;this.y=y;} module.exports = Point.bind(null,3);");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let prior = std::fs::read(&executable).unwrap();
    fixture.write("main.ts", b"class Unsupported { accessor value = 1; }");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("main.ts:1:21: auto-accessors are not compiled yet"),
        "{err}"
    );
    assert_eq!(std::fs::read(&executable).unwrap(), prior);
}

#[test]
fn builds_native_object_builtins_and_object_global_writes_throw() {
    let fixture = app(
        "const settings = Object.freeze(Object.assign(Object.create(null), {theme: 'dark'})); const keys = Object.keys({a: 1, b: 2}); if (keys.join() !== 'a,b' || !Object.isFrozen(settings) || Object.getPrototypeOf(settings) !== null) throw 'wrong native objects';",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    fixture.write("main.ts", b"Object = null;");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let replaced = Command::new(&executable).output().unwrap();
    assert!(!replaced.status.success(), "{replaced:?}");
    assert!(
        String::from_utf8_lossy(&replaced.stderr)
            .contains("TypeError: Replacing the global Object is not supported natively"),
        "{replaced:?}"
    );
}

#[test]
fn builds_native_symbol_protocols_and_symbol_global_writes_throw() {
    let fixture = app(
        "const key = Symbol('key'); const values = {[Symbol.iterator]() {let count=0;return {next() {count++;return {value:count,done:count>2};}};}}; const data = {[key]:42}; if ([...values].join(',') !== '1,2' || data[key] !== 42 || Symbol.keyFor(Symbol.for('same')) !== 'same') throw 'wrong native symbols';",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    fixture.write("main.ts", b"Symbol = () => 0;");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let replaced = Command::new(&executable).output().unwrap();
    assert!(!replaced.status.success(), "{replaced:?}");
    assert!(
        String::from_utf8_lossy(&replaced.stderr)
            .contains("TypeError: Replacing the global Symbol is not supported natively"),
        "{replaced:?}"
    );
}

#[test]
fn builds_numeric_globals_and_global_replacements_throw() {
    let fixture = app(
        "if (parseInt('17x',8) !== 15 || parseFloat('-.5e2tail') !== -50 || isNaN('x') !== true || isFinite('42') !== true || 1/parseInt('-0') !== -Infinity) throw 'wrong numeric globals';",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    fixture.write("main.ts", b"parseInt = () => 0;");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let replaced = Command::new(&executable).output().unwrap();
    assert!(!replaced.status.success(), "{replaced:?}");
    assert!(
        String::from_utf8_lossy(&replaced.stderr)
            .contains("TypeError: Replacing the global parseInt is not supported natively"),
        "{replaced:?}"
    );
}

#[test]
fn builds_cached_native_json_data_and_preserves_output_on_json_source_errors() {
    let fixture = app(
        "import data from './data.json'; import value from 'entry'; if (data !== value.data || data.answer !== 42 || data.__proto__.flag !== 7 || data.text.length !== 2 || value.flag !== true) throw 'wrong native JSON';",
    );
    fixture.write(
        "node_modules/entry/package.json",
        br#"{"main":"index.cjs"}"#,
    );
    fixture.write("node_modules/entry/index.cjs", b"'use strict'; exports.data = require('../../data.json'); exports.data.answer++; exports.flag = require('../../flag.json');");
    fixture.write("data.json", "\u{feff}{\"answer\":41,\"__proto__\":{\"old\":1},\"__proto__\":{\"flag\":7},\"text\":\"\\ud800\\udfff\"}".as_bytes());
    fixture.write("flag.json", b"true");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let executable = fixture.path("native-output/native-test.exe");
    std::fs::remove_file(fixture.path("data.json")).unwrap();
    std::fs::remove_file(fixture.path("flag.json")).unwrap();
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let prior = std::fs::read(&executable).unwrap();
    fixture.write("data.json", "\u{feff}{\n \"資料\": @\n}".as_bytes());
    fixture.write("flag.json", b"true");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(err.contains("data.json:2:8: invalid JSON:"), "{err}");
    assert_eq!(std::fs::read(&executable).unwrap(), prior);
}

#[test]
fn native_builtin_resolution_returns_names_without_loading_platform_modules() {
    let fixture = app("import names from './names.cjs'; console.log(JSON.stringify(names));");
    let names = [
        "path",
        "node:path",
        "path/win32",
        "node:path/posix",
        "fs",
        "node:fs",
        "inspector/promises",
        "_http_common",
        "stream/consumers",
        "node:test",
        "node:test/reporters",
        "node:sea",
        "node:sqlite",
        "keytar",
        "electron",
        "v8",
    ];
    let source = |names: &[&str]| {
        format!(
            "'use strict'; module.exports = [{}];",
            names
                .iter()
                .map(|name| format!("require.resolve({})", serde_json::to_string(name).unwrap()))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    fixture.write("names.cjs", source(&names).as_bytes());
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap().trim(),
        serde_json::to_string(&names).unwrap()
    );
    let mut dlls = imported_dlls(&executable);
    dlls.sort();
    assert_eq!(
        dlls,
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll"
        ]
    );

    fixture.write(
        "shadow.cjs",
        b"invalid source; resolution must not compile it",
    );
    fixture.write("node_modules/test/package.json", br#"{"main":"index.cjs"}"#);
    fixture.write(
        "node_modules/test/index.cjs",
        b"invalid user package; never load",
    );
    let config = serde_json::json!({"main": {
        "build": {"rollupOptions": {"input": "main.ts"}},
        "resolve": {"alias": {"#prefixed": "node:path", "#plain": "fs",
            "path": fixture.path("shadow.cjs").to_str().unwrap()}}
    }});
    fixture.write(
        "electron.vite.config.ts",
        format!("export default {config};").as_bytes(),
    );
    fixture.write(
        "names.cjs",
        source(&["#prefixed", "#plain", "path", "test", "node:test"]).as_bytes(),
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let expected = serde_json::to_string(&[
        "node:path".to_owned(),
        "fs".to_owned(),
        nativeboat_bundler::resolve::realpath(&fixture.path("shadow.cjs"))
            .to_str()
            .unwrap()
            .into(),
        nativeboat_bundler::resolve::realpath(&fixture.path("node_modules/test/index.cjs"))
            .to_str()
            .unwrap()
            .into(),
        "node:test".to_owned(),
    ])
    .unwrap();
    for file in ["names.cjs", "shadow.cjs", "node_modules/test/index.cjs"] {
        std::fs::remove_file(fixture.path(file)).unwrap();
    }
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(String::from_utf8(executed.stdout).unwrap().trim(), expected);
    let prior = std::fs::read(&executable).unwrap();
    // A missing module's name fails the build when it is resolved by name.
    fixture.write(
        "names.cjs",
        b"'use strict'; require.resolve('node:missing-native-module');",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("names.cjs:1:"), "{err}");
    assert_eq!(std::fs::read(&executable).unwrap(), prior);
    // Requiring it builds, and throws MODULE_NOT_FOUND when it runs, as in
    // Node; a guarded require gives no warning.
    fixture.write(
        "names.cjs",
        b"'use strict'; try { require('node:missing-native-module'); } catch (error) { module.exports = [error.code]; }",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap().trim(),
        "[\"MODULE_NOT_FOUND\"]"
    );
}

#[test]
fn resolves_native_commonjs_paths_without_loading_or_runtime_file_access() {
    let fixture = app("");
    fixture.write(
        "node_modules/entry/package.json",
        br#"{"main":"index.cjs"}"#,
    );
    fixture.write("node_modules/entry/index.cjs", "'use strict'; module.exports = [require.resolve('choice'), require.resolve('../choice/資源📒.json'), require.resolve('@asset'), require.resolve('esm'), require.resolve('opaque'), require('actual')];".as_bytes());
    fixture.write(
        "node_modules/choice/package.json",
        br#"{"exports":{"import":"./wrong.mjs","require":"./\u8cc7\u6e90\ud83d\udcd2.json"}}"#,
    );
    fixture.write(
        "node_modules/choice/資源📒.json",
        b"unparseable JSON; resolution must not load this",
    );
    fixture.write("node_modules/choice/wrong.mjs", b"wrong import condition");
    fixture.write(
        "node_modules/esm/package.json",
        br#"{"type":"module","main":"bad.mjs"}"#,
    );
    fixture.write(
        "node_modules/esm/bad.mjs",
        b"invalid source ! never parse or evaluate",
    );
    fixture.write(
        "node_modules/opaque/package.json",
        br#"{"main":"addon.node"}"#,
    );
    fixture.write("node_modules/opaque/addon.node", &[0xff, 0]);
    fixture.write(
        "node_modules/actual/package.json",
        br#"{"main":"index.cjs"}"#,
    );
    fixture.write(
        "node_modules/actual/index.cjs",
        b"'use strict'; module.exports = 7;",
    );
    fixture.write("tsconfig.json", "{\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"@asset\":[\"node_modules/choice/資源📒.json\"]}}}".as_bytes());
    let path_literal = |path: &str| {
        serde_json::to_string(
            &nativeboat_bundler::resolve::realpath(&fixture.path(path))
                .to_str()
                .unwrap(),
        )
        .unwrap()
    };
    fixture.write("main.ts", format!("import value from 'entry'; if (value[0] !== {asset} || value[1] !== value[0] || value[2] !== value[0] || value[3] !== {esm} || value[4] !== {opaque} || value[5] !== 7) throw 'wrong native resolution';", asset=path_literal("node_modules/choice/資源📒.json"), esm=path_literal("node_modules/esm/bad.mjs"), opaque=path_literal("node_modules/opaque/addon.node")).as_bytes());
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let executable = fixture.path("native-output/native-test.exe");
    for path in [
        "node_modules/choice/資源📒.json",
        "node_modules/esm/bad.mjs",
        "node_modules/opaque/addon.node",
        "node_modules/actual/index.cjs",
    ] {
        std::fs::remove_file(fixture.path(path)).unwrap();
    }
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    fixture.write(
        "node_modules/entry/index.cjs",
        b"'use strict';\nrequire('electron');",
    );
    fixture.write("main.ts", b"import 'entry';");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let prior = std::fs::read(&executable).unwrap();
    fixture.write(
        "node_modules/entry/index.cjs",
        b"'use strict';\nrequire.resolve('missing-native-path');",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(err.contains("index.cjs:2:17:"), "{err}");
    assert!(err.contains("missing-native-path"), "{err}");
    assert_eq!(std::fs::read(&executable).unwrap(), prior);
}

#[test]
fn compiles_commonjs_with_require_conditions_canonical_cache_and_source_errors() {
    let fixture = app(
        "import value from 'entry'; if (value !== 26 || Math.sumPrecise([1e16,1,-1e16]) !== 1 || Math.f16round(1.0004882812500002) !== 1.0009765625) throw 'wrong compiled CommonJS';",
    );
    fixture.write(
        "node_modules/entry/package.json",
        br#"{"main":"index.cjs"}"#,
    );
    fixture.write("node_modules/entry/index.cjs", b"'use strict'; const a = require('choice'); const b = require('./../choice/value.cjs'); module.exports = a.next(5.9) + b.next(6.4) + require('@native').extra;");
    fixture.write(
        "node_modules/choice/package.json",
        br#"{"exports":{"import":"./wrong.js","require":"./value.cjs"}}"#,
    );
    fixture.write(
        "node_modules/choice/wrong.js",
        b"throw 'wrong require conditions';",
    );
    fixture.write(
        "node_modules/choice/value.cjs",
        b"'use strict'; let n = 0; module.exports = {next: function() { return ++n + Math.floor(arguments[0]); }};",
    );
    fixture.write(
        "tsconfig.json",
        br#"{"compilerOptions":{"baseUrl":".","paths":{"@native":["src/value.cts"]}}}"#,
    );
    fixture.write("src/value.cts", b"'use strict'; exports.extra = 12;");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        Command::new(fixture.path("native-output/native-test.exe"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let renderer = graph(
        &project,
        &project.main.source,
        &project.aliases,
        Target::RendererImport,
        &CompileOptions::default(),
    )
    .unwrap();
    assert!(renderer.rust_source().contains("value.cjs"));
    assert!(!renderer.rust_source().contains("wrong.js"));
    let prior = std::fs::read(fixture.path("native-output/native-test.exe")).unwrap();
    fixture.write(
        "node_modules/entry/index.cjs",
        b"'use strict'; arguments[1]('choice');",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 1);
    assert!(err.contains("CommonJS wrapper arguments access"), "{err}");
    assert_eq!(
        std::fs::read(fixture.path("native-output/native-test.exe")).unwrap(),
        prior
    );
    fixture.write(
        "node_modules/entry/index.cjs",
        b"'use strict';\nrequire('electron');\nimport('unsupported-lazy-module');",
    );
    let (_, _, err) = run(&fixture, &[]);
    assert!(!err.contains("native platform module 'electron'"), "{err}");
    assert!(err.contains("unsupported-lazy-module"), "{err}");
    assert!(!fixture.path("dist").exists());
}

#[test]
fn reports_source_blockers_before_creating_output_or_running_scripts() {
    let fixture = app(
        "\nimport 'missing'; import 'electron';\nclass Unsupported { accessor x = 1; }\nconst a = import('some-package');",
    );
    let (code, out, err) = run(&fixture, &[]);
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert!(err.contains("main.ts:2:"), "{err}");
    assert!(!err.contains("native platform module 'electron'"), "{err}");
    assert!(err.contains("cannot resolve \"missing\""), "{err}");
    assert!(
        err.contains("cannot resolve dynamic import module 'some-package'"),
        "{err}"
    );
    assert!(err.contains("auto-accessors"), "{err}");
    assert!(!fixture.path("dist").exists());
    let empty = Fixture::new(&[]);
    assert!(run(&empty, &[]).2.contains("cannot read"));
    for removed in ["--allow-unsupported", "--runtime", "--no-scripts"] {
        assert_eq!(run(&fixture, &[removed]).0, 2, "{removed}");
    }
}

#[test]
fn compiles_finite_template_dynamic_imports_as_lazy_literal_edges() {
    let fixture = app(
        "import { loadLanguage } from './loadLanguage.ts';\nloadLanguage('one').then(module => console.log(module.language));",
    );
    fixture.write(
        "loadLanguage.ts",
        b"export const loadLanguage = (lang: string) => import(`./languages/prism-${lang}.js`);",
    );
    fixture.write("languages/prism-one.js", b"export const language = 'one';");
    fixture.write("languages/prism-two.js", b"export const language = 'two';");
    fixture.write(
        "languages/prism-one.min.js",
        b"class Unsupported { accessor value = 1; }",
    );

    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"one\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn native_compilation_leaves_packaging_steps_and_update_feeds_alone() {
    let fixture = app("export const native = true;");
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    project.layout = Layout::ElectronVite;
    project.build_steps = vec![nativeboat_project::scripts::Step {
        script: "build".into(),
        command: "electron-vite build".into(),
        optional: false,
    }];
    project.updater_config = Some(serde_json::json!({ "provider": "generic" }));

    let errors = compile(&project).errors.join("\n");
    assert!(!errors.contains("updater configuration"), "{errors}");

    // Packaging steps run before compiling; they are no compilation errors.
    project.build_steps[0].command = "tsx generate-native-inputs.ts".into();
    assert!(compile(&project).errors.is_empty());
}

#[test]
fn electron_host_exposes_only_implemented_named_exports() {
    let fixture = app(
        "import { app, BrowserWindow, clipboard, dialog, ipcMain, Menu, MenuItem, shell } from 'electron'; if (!app.isReady() || typeof shell.openPath !== 'function' || typeof clipboard.readText !== 'function' || typeof clipboard.writeText !== 'function' || typeof clipboard.readBuffer !== 'function' || typeof clipboard.has !== 'function' || typeof dialog.showOpenDialog !== 'function' || typeof dialog.showSaveDialog !== 'function' || typeof dialog.showMessageBox !== 'function' || typeof dialog.showErrorBox !== 'function' || typeof Menu.buildFromTemplate !== 'function' || typeof MenuItem !== 'function') throw 'not ready'; ipcMain.on('native-event', value => { if (value !== 42) throw 'bad event'; }); ipcMain.handle('native-request', () => 42); ipcMain.emit('native-event', 42); new BrowserWindow();",
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let supported = graph(
        &project,
        &project.main.source,
        &project.aliases,
        Target::RendererImport,
        &CompileOptions::default(),
    )
    .unwrap();
    assert!(supported.native_dependencies().contains("electron"));

    fixture.write(
        "main.ts",
        b"import { app, BaseWindow } from 'electron'; app.isReady(); new BaseWindow();",
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let errors = graph(
        &project,
        &project.main.source,
        &project.aliases,
        Target::RendererImport,
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("has no export 'BaseWindow'")),
        "{errors:?}"
    );
}

#[test]
fn generated_native_keymap_module_runs_without_node_or_javascript_engine() {
    let fixture = app(
        "import { getKeyMap, getCurrentKeyboardLayout, onDidChangeKeyboardLayout, isISOKeyboard } from 'native-keymap'; const map = getKeyMap(); if (Object.keys(map).length !== 146 || map.KeyA.vkey === 'VK_UNKNOWN' || typeof map.KeyA.value !== 'string' || typeof map.KeyA.withShiftAltGr !== 'string') throw 'keyboard map failed'; const layout = getCurrentKeyboardLayout(); if (!layout || typeof layout.name !== 'string' || !layout.name || typeof layout.id !== 'string' || typeof layout.text !== 'string' || !layout.text) throw 'keyboard layout failed'; let changed = 0; onDidChangeKeyboardLayout(() => changed++); if (changed !== 0 || isISOKeyboard() !== undefined) throw 'keyboard event API failed'; console.log('native-keymap-ready');",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"native-keymap-ready\n");
    assert!(executed.stderr.is_empty());
    let dlls = imported_dlls(&executable);
    assert!(dlls.iter().any(|name| name == "user32.dll"), "{dlls:?}");
    for forbidden in ["node.dll", "electron.dll", "v8.dll", "boa.dll"] {
        assert!(!dlls.iter().any(|name| name == forbidden), "{dlls:?}");
    }
}

#[test]
fn generated_native_keytar_module_runs_without_node_or_javascript_engine() {
    let fixture = app(
        "import keytar from 'keytar'; const expected = ['deletePassword', 'findCredentials', 'findPassword', 'getPassword', 'setPassword']; if (Object.keys(keytar).sort().join(',') !== expected.join(',') || Object.values(keytar).some(method => typeof method !== 'function')) throw 'keytar API failed'; console.log('native-keytar-ready');",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"native-keytar-ready\n");
    assert!(executed.stderr.is_empty());
    let dlls = imported_dlls(&executable);
    for forbidden in [
        "node.dll",
        "electron.dll",
        "v8.dll",
        "boa.dll",
        "advapi32.dll",
    ] {
        assert!(!dlls.iter().any(|name| name == forbidden), "{dlls:?}");
    }
}

#[test]
fn generated_ced_detector_runs_without_a_javascript_engine_or_crt() {
    let fixture = app(
        "import ced from 'ced'; if(typeof ced!=='function') throw Error('ced export'); if(ced(Buffer.alloc(0))!=='ASCII') throw Error('empty input'); if(ced(Buffer.from('plain text'))!=='ASCII-7-bit') throw Error('ASCII'); if(ced(Buffer.from('µ κ α'))!=='UTF8') throw Error('UTF-8'); if(ced(Buffer.from([255,254,65,0]))!=='UTF-16LE') throw Error('UTF-16LE'); let rejected=false; try { ced(new Uint8Array([1])); } catch(error) { rejected=error instanceof TypeError && error.message==='Expected a buffer'; } if(!rejected) throw Error('Buffer validation'); console.log('ced-native-ok');",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap().trim(),
        "ced-native-ok"
    );
    let dlls = imported_dlls(&executable);
    assert!(!dlls.iter().any(|name| {
        let lower = name.to_ascii_lowercase();
        lower.contains("boa")
            || lower.contains("v8")
            || lower.contains("node")
            || lower.contains("electron")
            || lower.contains("ucrt")
            || lower.contains("msvcp")
    }));
}

#[test]
fn generated_http_module_reports_native_request_errors_without_an_engine() {
    let fixture = app(
        "import http from 'http'; import https from 'https'; try { http.request('not a URL'); } catch (error) { console.log(`native-http-error ${error.code}`); } https.get('https://127.0.0.1:1/').on('error', (error) => console.log(`native-https-error ${error.code}`));",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "native-http-error ERR_INVALID_URL\nnative-https-error ECONNREFUSED\n"
    );
    assert!(executed.stderr.is_empty());
    let dlls = imported_dlls(&executable);
    assert!(dlls.iter().any(|name| name == "winhttp.dll"), "{dlls:?}");
    for forbidden in ["boa.dll", "electron.dll", "node.dll", "v8.dll"] {
        assert!(!dlls.iter().any(|name| name == forbidden), "{dlls:?}");
    }
}

/// OpenSSL from PATH or Git for Windows, which serves HTTPS with a
/// certificate made for a test.
fn openssl() -> PathBuf {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join("openssl.exe"))
        .chain([PathBuf::from(r"C:\Program Files\Git\usr\bin\openssl.exe")])
        .find(|candidate| candidate.is_file())
        .expect("OpenSSL serves the HTTPS test; Git for Windows has one")
}

#[test]
fn generated_https_responses_stream_and_take_untrusted_certificates_when_asked() {
    let fixture = app(
        "import https from 'https';
const url = `https://127.0.0.1:${process.env.NATIVE_TEST_PORT}/large.bin`;
const get = (options) => new Promise((resolve) => {
  const request = https.get(url, options, (response) => {
    let chunks = 0, first = 0, total = 0, paused = false;
    response.on('data', (chunk) => {
      chunks += 1;
      total += chunk.length;
      if (chunks === 1) {
        first = total;
        if (options.destroy) { response.destroy(); return; }
        response.pause();
        paused = true;
        setTimeout(() => { paused = false; response.resume(); }, 200);
      } else if (paused) {
        resolve('data while paused');
      }
    });
    response.on('aborted', () => resolve(`aborted ${first > 0}`));
    response.on('end', () => resolve(`${response.statusCode} ${total} ${chunks > 1 && first < total}`));
  });
  request.on('error', (error) => resolve(`error ${error.code}`));
});
(async () => {
  console.log(await get({}));
  console.log(await get({ rejectUnauthorized: false }));
  console.log(await get({ agent: new https.Agent({ rejectUnauthorized: false }) }));
  console.log(await get({ rejectUnauthorized: false, destroy: true }));
  process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0';
  console.log(await get({}));
})();",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let site = fixture.path("site");
    std::fs::create_dir_all(&site).unwrap();
    // Letters only: OpenSSL serves files as text, ending at Ctrl+Z.
    let large: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|index| b'a' + (index % 26) as u8)
        .collect();
    std::fs::write(site.join("large.bin"), &large).unwrap();
    let openssl = openssl();
    let made = Command::new(&openssl)
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "key.pem",
            "-out",
            "cert.pem",
            "-subj",
            "/CN=127.0.0.1",
            "-days",
            "1",
        ])
        .current_dir(&site)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Command::new(&openssl)
        .args([
            "s_server", "-quiet", "-WWW", "-cert", "cert.pem", "-key", "key.pem",
        ])
        .arg("-accept")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(&site)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let listening = (0..100).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
    });
    let executable = fixture.path("native-output/native-test.exe");
    let executed = listening.then(|| {
        Command::new(&executable)
            .env("NATIVE_TEST_PORT", port.to_string())
            .output()
            .unwrap()
    });
    let _ = server.kill();
    let _ = server.wait();
    let executed = executed.expect("OpenSSL serves HTTPS");
    assert!(executed.status.success(), "{executed:?}");
    let size = large.len();
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        format!(
            "error CERT_INVALID\n200 {size} true\n200 {size} true\naborted true\n200 {size} true\n"
        )
    );
}

#[test]
fn generated_electron_net_fetch_rejects_invalid_urls_without_an_engine() {
    let fixture = app(
        "import { net, session } from 'electron'; session.defaultSession.setProxy({ mode: 'system' }).then(() => net.fetch('not a URL', { redirect: 'error' })).catch(error => { if (error.code !== 'ERR_INVALID_URL') throw error; console.log('native-electron-fetch'); });",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"native-electron-fetch\n");
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        !["boa.dll", "electron.dll", "node.dll", "v8.dll"].contains(&name.as_str())
    }));
}

#[test]
fn generated_electron_net_requests_follow_electron_redirect_modes() {
    let fixture = app(
        "import { net, session } from 'electron'; import http from 'http';
let port;
const server = http.createServer((req, res) => {
  let body = '';
  req.on('data', (chunk) => { body += chunk; });
  req.on('end', () => {
    if (req.url === '/moved') { res.writeHead(302, { Location: '/data' }); res.end(); return; }
    if (req.url === '/see-other') { res.writeHead(303, { Location: `http://127.0.0.1:${port}/echo` }); res.end(); return; }
    res.setHeader('X-Agent', /Chrome\\//.test(req.headers['user-agent']) ? 'chrome' : String(req.headers['user-agent']));
    res.setHeader('X-Test', String(req.headers['x-test']));
    res.end(`${req.method} ${req.url} [${body}]`);
  });
});
const run = (options, onRedirect, send = (request) => request.end()) => new Promise((resolve) => {
  const events = [];
  const request = net.request(options);
  request.on('redirect', (status, method, url) => {
    events.push(`redirect ${status} ${method} ${url.replace(String(port), 'P')}`);
    if (onRedirect) onRedirect(request);
  });
  request.on('response', (response) => {
    let text = '';
    response.on('data', (chunk) => { text += chunk; });
    response.on('end', () => events.push(`response ${response.statusCode} ${response.headers['x-agent']} ${response.headers['x-test']} ${text}`));
  });
  request.on('abort', () => events.push('abort'));
  request.on('error', (error) => events.push(`error ${error.message}`));
  request.on('close', () => { events.push('close'); resolve(events.join(', ')); });
  send(request);
});
server.listen(0, '127.0.0.1', async () => {
  port = server.address().port;
  const base = `http://127.0.0.1:${port}`;
  console.log(session.fromPartition('electron-updater', { cache: false }) === session.defaultSession);
  try { session.fromPartition(); } catch (error) { console.log(error.constructor.name); }
  console.log(await run(`${base}/moved`));
  console.log(await run({ url: `${base}/moved`, redirect: 'manual' }, (request) => request.followRedirect()));
  console.log(await run({ url: `${base}/moved`, redirect: 'manual' }, (request) => request.abort()));
  console.log(await run({ url: `${base}/moved`, redirect: 'manual' }));
  console.log(await run({ url: `${base}/moved`, redirect: 'error' }));
  console.log(await run({ method: 'POST', protocol: 'http:', hostname: '127.0.0.1', port, path: '/see-other', headers: { 'X-Test': 'yes' } }));
  console.log(await run({ method: 'post', host: `127.0.0.1:${port}`, path: '/echo' }, null, (request) => { request.write('a'); request.end('b'); }));
  const probe = net.request(`${base}/data`);
  probe.setHeader('X-Test', 1);
  const header = probe.getHeader('x-test');
  probe.removeHeader('X-TEST');
  const removed = probe.getHeader('x-test');
  const before = JSON.stringify(probe.getUploadProgress());
  const failures = [];
  try { probe.followRedirect(); } catch (error) { failures.push(error.message); }
  probe.on('response', (response) => response.resume());
  const closed = new Promise((resolve) => probe.on('close', resolve));
  probe.end('xy');
  try { probe.setHeader('X-Late', 'no'); } catch (error) { failures.push(error.message); }
  try { probe.removeHeader('X-Test'); } catch (error) { failures.push(error.message); }
  console.log(`${header} ${removed} ${before} ${JSON.stringify(probe.getUploadProgress())} ${failures.join(' / ')}`);
  await closed;
  for (const options of [{ url: 'ftp://127.0.0.1/' }, { url: base, redirect: 'sometimes' }]) {
    try { net.request(options); } catch (error) { console.log(error.message); }
  }
  server.close(async () => console.log(await run(`${base}/data`)));
});",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "true\n\
         TypeError\n\
         redirect 302 GET http://127.0.0.1:P/data, response 200 chrome undefined GET /data [], close\n\
         redirect 302 GET http://127.0.0.1:P/data, response 200 chrome undefined GET /data [], close\n\
         redirect 302 GET http://127.0.0.1:P/data, abort, close\n\
         redirect 302 GET http://127.0.0.1:P/data, error Redirect was cancelled, close\n\
         error Attempted to redirect, but redirect policy was 'error', close\n\
         redirect 303 GET http://127.0.0.1:P/echo, response 200 chrome yes GET /echo [], close\n\
         response 200 chrome undefined POST /echo [ab], close\n\
         1 undefined {\"active\":false,\"started\":false,\"current\":0,\"total\":0} {\"active\":true,\"started\":true,\"current\":2,\"total\":2} \
         followRedirect() called, but was not waiting for a redirect / Can't set headers after they are sent / Can't remove headers after they are sent\n\
         Protocol \"ftp:\" not supported\n\
         redirect mode should be one of follow, error or manual\n\
         error net::ERR_CONNECTION_REFUSED, close\n"
    );
}

#[test]
fn settings_files_give_native_builds_their_feed() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-test","version":"1.0.0","main":"main.js","devDependencies":{"electron":"44"},"repository":"github:owner/repo","build":{"publish":"github"}}"#,
        ),
        ("main.js", "require('electron');"),
        (
            "nativeboat.json",
            r#"{"publish":{"provider":"generic","url":"http://127.0.0.1:1/native"},"later":true}"#,
        ),
    ]);
    let settings = fixture.path("nativeboat.json");
    let (code, out, err) = run(
        &fixture,
        &[
            "--out",
            "native-output",
            "--config",
            settings.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{out}{err}");
    let feed =
        std::fs::read_to_string(fixture.path("native-output/resources/app-update.yml")).unwrap();
    assert_eq!(
        feed,
        "provider: generic\nurl: http://127.0.0.1:1/native\nupdaterCacheDirName: native-test-updater\n"
    );
    let printed = format!("{out}{err}");
    assert!(!printed.contains("auto-updates use"), "{printed}");
    assert!(
        err.contains(
            "warning: the settings file gives \"later\", which this NativeBoat does not use"
        ),
        "{printed}"
    );
    // Without it, the build warns that updates come from the Electron feed.
    let (code, out, err) = run(&fixture, &["--out", "electron-feed"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(err.contains("warning: auto-updates use"), "{out}{err}");
    // A settings file that cannot be read stops the build.
    let missing = fixture.path("none.json");
    let (code, _, err) = run(
        &fixture,
        &["--out", "other", "--config", missing.to_str().unwrap()],
    );
    assert_eq!(code, 1);
    assert!(err.contains("cannot read the settings file"), "{err}");
}

#[test]
fn generated_installers_install_update_and_uninstall_native_builds() {
    let package = |version: &str, extra: &str| {
        format!(
            r#"{{"name":"native-test","version":"{version}","author":"Example","type":"module","main":"main.ts","devDependencies":{{"electron":"44"}},"build":{{"appId":"com.example.native-test",{extra}"nsis":{{"deleteAppDataOnUninstall":true}}}},"nativeboat":{{"publish":{{"provider":"generic","url":"http://127.0.0.1:1/updates"}}}}}}"#
        )
    };
    let fixture = Fixture::new(&[
        ("package.json", &package("1.0.0", r#""extraFiles":["old.txt"],"#)),
        ("old.txt", "from the first version"),
        (
            "main.ts",
            "import { app } from 'electron'; import { spawn } from 'child_process'; import fs from 'fs'; import path from 'path';
fs.writeFileSync(path.join(process.env.NATIVEBOAT_SETUP_STATE, 'ran.txt'), `${app.getVersion()} ${path.dirname(process.execPath)}`);
const update = process.env.NATIVEBOAT_TEST_UPDATE;
if (update && app.getVersion() === '1.0.0') spawn(update, ['--updated', '/S', '--force-run'], { detached: true, stdio: 'ignore' }).unref();",
        ),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "first"]);
    assert_eq!(code, 0, "{out}{err}");
    let first = fixture.path("first-setup/native-test-Setup-1.0.0.exe");
    assert!(
        out.contains("Built installer ")
            && out.contains("first-setup\\native-test-Setup-1.0.0.exe")
            && first.is_file(),
        "{out}"
    );
    let feed = std::fs::read_to_string(fixture.path("first/resources/app-update.yml")).unwrap();
    assert_eq!(
        feed,
        "provider: generic\nurl: http://127.0.0.1:1/updates\nupdaterCacheDirName: native-test-updater\n"
    );
    let latest = std::fs::read_to_string(fixture.path("first-setup/latest.yml")).unwrap();
    let size = std::fs::metadata(&first).unwrap().len();
    assert!(
        latest
            .starts_with("version: 1.0.0\nfiles:\n- url: native-test-Setup-1.0.0.exe\n  sha512: "),
        "{latest}"
    );
    assert!(
        latest.contains(&format!(
            "\n  size: {size}\npath: native-test-Setup-1.0.0.exe\n"
        )),
        "{latest}"
    );

    // A silent install into a named folder, with the registry, shortcuts and
    // known folders kept in a test folder.
    let state = fixture.path("state");
    let installed = fixture.path("installed app");
    std::fs::create_dir_all(state.join("RoamingAppData/native-test")).unwrap();
    std::fs::write(state.join("RoamingAppData/native-test/settings.json"), "{}").unwrap();
    let status = Command::new(&first)
        .arg("/S")
        .raw_arg(format!("/D={}", installed.display()))
        .env("NATIVEBOAT_SETUP_STATE", &state)
        .status()
        .unwrap();
    assert!(status.success(), "{status:?}");
    for file in [
        "native-test.exe",
        "old.txt",
        "resources/app-update.yml",
        "Uninstall native-test.exe",
    ] {
        assert!(installed.join(file).is_file(), "{file}");
    }
    assert!(state.join("Programs/native-test.lnk").is_file());
    assert!(state.join("Desktop/native-test.lnk").is_file());
    let registry = std::fs::read_to_string(state.join("registry.txt")).unwrap();
    assert!(
        registry.contains(&format!(
            "Software\\com.example.native-test\\InstallLocation={}\n",
            installed.display()
        )),
        "{registry}"
    );
    assert!(
        registry.contains("Uninstall\\com.example.native-test\\DisplayName=native-test 1.0.0\n"),
        "{registry}"
    );
    assert!(registry.contains("\\Publisher=Example\n"), "{registry}");
    assert!(
        !state.join("ran.txt").exists(),
        "silent installs start nothing"
    );

    // The second version's installer, which the first version runs as
    // electron-updater does, installs into its folder once it exits.
    fixture.write("package.json", package("2.0.0", "").as_bytes());
    // The second version no longer has the first one's file.
    std::fs::remove_file(fixture.path("old.txt")).unwrap();
    let (code, out, err) = run(&fixture, &["--out", "second"]);
    assert_eq!(code, 0, "{out}{err}");
    let second = fixture.path("second-setup/native-test-Setup-2.0.0.exe");
    let status = Command::new(installed.join("native-test.exe"))
        .env("NATIVEBOAT_SETUP_STATE", &state)
        .env("NATIVEBOAT_TEST_UPDATE", &second)
        .status()
        .unwrap();
    assert!(status.success(), "{status:?}");
    let ran = format!("2.0.0 {}", installed.display());
    let updated = (0..300).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::fs::read_to_string(state.join("ran.txt")).is_ok_and(|text| text == ran)
            && !installed.join("old.txt").exists()
    });
    assert!(
        updated,
        "{:?}",
        std::fs::read_to_string(state.join("ran.txt"))
    );
    let registry = std::fs::read_to_string(state.join("registry.txt")).unwrap();
    assert!(
        registry.ends_with("NoRepair=dword:1\n") && registry.contains("DisplayVersion=2.0.0\n"),
        "{registry}"
    );

    // The uninstaller removes the files, shortcuts, registration and data.
    let status = Command::new(installed.join("Uninstall native-test.exe"))
        .args(["/currentuser", "/S"])
        .env("NATIVEBOAT_SETUP_STATE", &state)
        .status()
        .unwrap();
    assert!(status.success(), "{status:?}");
    let removed = (0..300).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(200));
        !installed.exists() && !state.join("RoamingAppData/native-test").exists()
    });
    assert!(
        removed,
        "{:?}",
        std::fs::read_dir(&installed).map(|entries| entries.count())
    );
    assert!(!state.join("Programs/native-test.lnk").exists());
    assert!(!state.join("Desktop/native-test.lnk").exists());
    assert_eq!(
        std::fs::read_to_string(state.join("registry.txt")).unwrap(),
        ""
    );
}

#[test]
fn generated_electron_app_runs_readiness_in_the_native_realm() {
    let fixture = app(
        "import { app, clipboard, dialog, ipcMain, Menu, MenuItem, nativeTheme, screen, shell } from 'electron'; if (!app.isReady() || typeof shell.openExternal !== 'function' || typeof clipboard.readText !== 'function' || typeof clipboard.readBuffer !== 'function' || typeof dialog.showOpenDialog !== 'function' || typeof Menu.buildFromTemplate !== 'function' || typeof MenuItem !== 'function') throw 'app is not ready'; const display = screen.getPrimaryDisplay(); if (display.bounds.width <= 0 || screen.getAllDisplays().length < 1) throw 'screen query failed'; const cursor = screen.getCursorScreenPoint(); if (!screen.getDisplayNearestPoint(cursor) || !screen.getDisplayMatching(display.bounds)) throw 'display selection failed'; let updates = 0; nativeTheme.on('updated', () => updates++); nativeTheme.themeSource = 'dark'; if (!nativeTheme.shouldUseDarkColors || updates !== 1) throw 'dark theme failed'; nativeTheme.themeSource = 'system'; if (updates !== 2 || typeof nativeTheme.shouldUseDarkColors !== 'boolean') throw 'system theme failed'; ipcMain.on('ready-check', value => { if (value !== 7) throw 'bad ipc event'; }); ipcMain.emit('ready-check', 7); ipcMain.handle('readiness-query', () => true); app.whenReady().then(() => console.log('native-ready'));",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"native-ready\n");
    assert!(executed.stderr.is_empty());
    let dlls = imported_dlls(&fixture.path("native-output/native-test.exe"));
    for required in ["advapi32.dll", "shcore.dll", "user32.dll"] {
        assert!(dlls.iter().any(|name| name == required), "{dlls:?}");
    }
    assert!(dlls.iter().all(|name| {
        !["boa.dll", "electron.dll", "node.dll", "v8.dll"].contains(&name.as_str())
    }));
}

#[test]
fn generated_electron_apps_hand_later_instances_to_the_first() {
    let fixture = app("import { app } from 'electron'; \
         if (!app.requestSingleInstanceLock({ note: 'hi' })) { console.log('second'); app.quit(); } else { \
         setInterval(() => {}, 1000); \
         app.on('ready', () => console.log('ready')); \
         app.on('second-instance', (event, argv, cwd, data) => { \
           console.log(`got ${argv.slice(1).join(' ')} ${typeof cwd} ${data.note} ${app.hasSingleInstanceLock()}`); \
           app.releaseSingleInstanceLock(); console.log(`released ${app.hasSingleInstanceLock()}`); app.quit(); }); \
           app.on('before-quit', () => console.log('before-quit')); \
           app.on('will-quit', () => console.log('will-quit')); \
           app.on('quit', (event, code) => console.log(`quit ${code}`)); }");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    // Instances meet through their user data directory.
    let app_data = fixture.path("app-data");
    std::fs::create_dir(&app_data).unwrap();
    let mut first = Command::new(&executable)
        .arg("first.md")
        .env("APPDATA", &app_data)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = std::io::BufReader::new(first.stdout.take().unwrap());
    let mut line = String::new();
    std::io::BufRead::read_line(&mut output, &mut line).unwrap();
    assert_eq!(line, "ready\n");
    let second = Command::new(&executable)
        .arg("later.md")
        .env("APPDATA", &app_data)
        .output()
        .unwrap();
    assert!(second.status.success(), "{second:?}");
    assert_eq!(second.stdout, b"second\n");
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut output, &mut rest).unwrap();
    assert!(first.wait().unwrap().success());
    assert_eq!(
        rest,
        "got later.md string hi true\nreleased false\nbefore-quit\nwill-quit\nquit 0\n"
    );
}

#[test]
fn generated_electron_apps_quit_with_pending_timers_and_closing_windows_can_refuse() {
    let fixture = app("import { app, BrowserWindow } from 'electron'; \
         setInterval(() => {}, 1000); \
         app.on('ready', () => { \
           const window = new BrowserWindow({ show: false }); \
           let asked = 0; \
           window.on('close', (event) => { asked++; console.log(`close ${asked}`); if (asked === 1) { event.preventDefault(); setTimeout(() => app.quit(), 50); } }); \
           window.on('closed', () => console.log('closed')); \
           app.on('window-all-closed', () => console.log('window-all-closed')); \
           app.on('before-quit', () => console.log('before-quit')); \
           app.on('will-quit', () => console.log('will-quit')); \
           app.quit(); });");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    // The first close is refused, which cancels that quit; the second quit
    // closes the window and ends the app without `window-all-closed`.
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "before-quit\nclose 1\nbefore-quit\nclose 2\nclosed\nwill-quit\n"
    );
}

#[test]
fn generated_electron_crash_reporter_starts_without_an_engine() {
    let fixture = app(
        "import { crashReporter } from 'electron'; crashReporter.start({ companyName: 'Nativeboat', productName: 'MarkText', uploadToServer: false, compress: true }); console.log(typeof crashReporter.start);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"function\n");
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        ![
            "boa.dll",
            "electron.dll",
            "node.dll",
            "v8.dll",
            "dbghelp.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn generated_electron_native_image_decodes_png_without_an_engine() {
    let fixture = app(
        "import { clipboard, nativeImage } from 'electron'; const png = new Uint8Array([137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,1,0,0,0,1,8,6,0,0,0,31,21,196,137,0,0,0,16,73,68,65,84,120,1,1,5,0,250,255,0,200,100,50,128,5,53,1,223,33,83,108,72,0,0,0,0,73,69,78,68,174,66,96,130]); const image = nativeImage.createFromBuffer(png); const size = image.getSize(); const pixels = image.toBitmap(); if (image.isEmpty() || size.width !== 1 || size.height !== 1 || pixels.length !== 4 || pixels[0] !== 25 || pixels[1] !== 50 || pixels[2] !== 100 || pixels[3] !== 128 || typeof clipboard.writeImage !== 'function') throw 'native image failed'; console.log(`${size.width}x${size.height}:${pixels[0]},${pixels[1]},${pixels[2]},${pixels[3]}`);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"1x1:25,50,100,128\n");
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        !["boa.dll", "electron.dll", "node.dll", "v8.dll"].contains(&name.as_str())
    }));
}

#[test]
fn generated_app_runs_packaged_native_zlib_without_node_or_engine() {
    // Stored DEFLATE and zlib-wrapped blocks containing `native zlib`.
    let fixture = app(
        "import { createUnzip, inflateRawSync } from 'zlib'; const raw = new Uint8Array([1, 11, 0, 244, 255, 110, 97, 116, 105, 118, 101, 32, 122, 108, 105, 98]); const value = inflateRawSync(raw); if (String.fromCharCode(...value) !== 'native zlib') throw 'raw inflater failed'; const stream = createUnzip(); let output = ''; stream.on('data', chunk => { output += String.fromCharCode(...chunk); }); stream.on('end', () => { if (output !== 'native zlib') throw 'unzip stream failed'; console.log(output); streamed(); }); const wrapped = new Uint8Array([120, 1, 1, 11, 0, 244, 255, 110, 97, 116, 105, 118, 101, 32, 122, 108, 105, 98, 26, 131, 4, 89]); stream.write(wrapped.subarray(0, 7)); stream.end(wrapped.subarray(7));
import { constants, codes, Z_SYNC_FLUSH, Gzip, createGunzip, deflateSync, inflateSync, crc32 } from 'zlib';
if (constants.Z_SYNC_FLUSH !== 2 || Z_SYNC_FLUSH !== 2 || codes.Z_BUF_ERROR !== -5 || codes['-5'] !== 'Z_BUF_ERROR') throw 'constants';
if (crc32('hello') !== 907060870 || crc32('lo', crc32('hel')) !== 907060870) throw 'crc32';
const packed = deflateSync('partial zlib data, partial zlib data');
if (String(inflateSync(packed.subarray(0, packed.length - 4), { finishFlush: Z_SYNC_FLUSH })) !== 'partial zlib data, partial zlib data') throw 'partial input';
let code = '';
try { inflateSync(packed.subarray(0, 5)); } catch (error) { code = `${error.code} ${error.errno} ${error.message}`; }
if (code !== 'Z_BUF_ERROR -5 unexpected end of file') throw code;
function streamed() {
  const gzip = Gzip();
  if (!(gzip instanceof Gzip) || Gzip.name !== 'Gzip') throw 'class';
  let text = '';
  gzip.pipe(createGunzip()).on('data', (chunk) => { text += chunk; }).on('end', () => console.log(text));
  gzip.end('streamed');
}",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"native zlib\nstreamed\n");
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        !["boa.dll", "electron.dll", "node.dll", "v8.dll"].contains(&name.as_str())
    }));
}

#[test]
fn generated_app_serves_and_requests_http_over_native_sockets() {
    let fixture = app(
        "import http from 'http'; import net from 'net';
const echo = net.createServer((socket) => socket.pipe(socket));
echo.listen(0, '127.0.0.1', () => {
  const client = net.connect(echo.address().port, '127.0.0.1', () => client.end('ping'));
  let echoed = '';
  client.setEncoding('utf8');
  client.on('data', (chunk) => { echoed += chunk; });
  client.on('end', () => {
    console.log(`echo ${echoed} ${client.remotePort === echo.address().port}`);
    echo.close(() => refused());
  });
});
function refused() {
  net.connect(echo.address()?.port || 1, '127.0.0.1').on('error', (error) => {
    console.log(`refused ${error.code} ${error.syscall}`);
    serve();
  });
}
function serve() {
  const server = http.createServer((req, res) => {
    let body = '';
    req.setEncoding('utf8');
    req.on('data', (chunk) => { body += chunk; });
    req.on('end', () => {
      if (req.url === '/stream') {
        res.writeHead(201, { 'Content-Type': 'text/plain' });
        res.write('a');
        setTimeout(() => res.end('b'), 10);
        return;
      }
      res.setHeader('X-Echo', req.headers['x-test']);
      res.end(`${req.method} ${req.url} ${body}`);
    });
  });
  server.listen(0, '127.0.0.1', () => {
    const { port } = server.address();
    const req = http.request({ port, host: '127.0.0.1', method: 'POST', path: '/echo?x=1', headers: { 'X-Test': 'yes' } }, (res) => {
      let text = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => { text += chunk; });
      res.on('end', () => {
        console.log(`${res.statusCode} ${res.headers['x-echo']} ${text}`);
        http.get(`http://127.0.0.1:${port}/stream`, (second) => {
          let more = '';
          second.on('data', (chunk) => { more += chunk; });
          second.on('end', () => {
            console.log(`${second.statusCode} ${second.headers['transfer-encoding']} ${more} ${second.req.reusedSocket}`);
            server.close(() => console.log('closed'));
          });
        });
      });
    });
    req.write('hello ');
    req.end('world');
  });
}",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "echo ping true\nrefused ECONNREFUSED connect\n200 yes POST /echo?x=1 hello world\n201 chunked ab false\nclosed\n"
    );
}

#[test]
fn generated_app_runs_web_streams() {
    let fixture = app(
        "import { ReadableStream, WritableStream, TransformStream, TextDecoderStream, CountQueuingStrategy } from 'stream/web';
import { Readable } from 'stream';
(async () => {
  const chunks = [];
  const encoder = new TextEncoder();
  const source = new ReadableStream({ start(controller) { controller.enqueue(encoder.encode('he')); controller.enqueue(encoder.encode('llo')); controller.close(); } });
  const upper = new TransformStream({ transform(chunk, controller) { controller.enqueue(chunk.toUpperCase()); } });
  await source.pipeThrough(new TextDecoderStream()).pipeThrough(upper).pipeTo(new WritableStream({ write(chunk) { chunks.push(chunk); } }, new CountQueuingStrategy({ highWaterMark: 2 })));
  const node = Readable.fromWeb(new ReadableStream({ pull(controller) { controller.enqueue('x'); controller.close(); } }), { encoding: 'utf8' });
  let fromWeb = '';
  for await (const chunk of node) fromWeb += chunk;
  const web = Readable.toWeb(Readable.from(['a', 'b']));
  let toWeb = '';
  for await (const chunk of web) toWeb += chunk;
  const reader = new ReadableStream({ start(controller) { controller.error(new TypeError('boom')); } }).getReader();
  let failed = '';
  try { await reader.read(); } catch (error) { failed = error.message; }
  const [left, right] = new ReadableStream({ start(controller) { controller.enqueue(1); controller.close(); } }).tee();
  const teed = [(await left.getReader().read()).value, (await right.getReader().read()).value];
  console.log(`${chunks.join('')} ${fromWeb} ${toWeb} ${failed} ${teed}`);
})();",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    // Node 22 prints the same for this program.
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "HELLO x ab boom 1,1\n"
    );
}

#[test]
fn generated_app_fetches_with_the_global_fetch_api() {
    let fixture = app(
        "import http from 'http';
import zlib from 'zlib';
const GlobalRequest = global.Request;
class AppRequest extends GlobalRequest {
  constructor(input, options) { super(input, options); this.app = true; }
}
const server = http.createServer((req, res) => {
  let body = '';
  req.on('data', (chunk) => { body += chunk; });
  req.on('end', () => {
    if (req.url === '/json') {
      res.setHeader('Content-Type', 'application/json');
      res.setHeader('Set-Cookie', ['a=1', 'b=2']);
      res.end(JSON.stringify({ ok: true }));
    } else if (req.url === '/gzip') {
      res.writeHead(200, { 'Content-Encoding': 'gzip' });
      res.end(zlib.gzipSync('compressed'));
    } else if (req.url === '/redirect') {
      res.writeHead(302, { Location: '/echo' });
      res.end();
    } else {
      res.end(`${req.method} ${req.url} ${req.headers['content-type'] || '-'} ${body}`);
    }
  });
});
server.listen(0, '127.0.0.1', async () => {
  const base = `http://127.0.0.1:${server.address().port}`;
  const out = [];
  const json = await fetch(`${base}/json`);
  out.push(`${json.status} ${json.ok} ${json.headers.get('content-type')} ${json.headers.getSetCookie().join(';')} ${JSON.stringify(await json.json())}`);
  const posted = await fetch(new AppRequest(`${base}/echo`, { method: 'POST', body: 'hello' }));
  out.push(await posted.text());
  const redirected = await fetch(`${base}/redirect`, { method: 'POST', body: 'x' });
  out.push(`${redirected.redirected} ${redirected.url.endsWith('/echo')} ${await redirected.text()}`);
  out.push(await (await fetch(`${base}/gzip`)).text());
  const form = new FormData();
  form.append('name', 'value');
  form.append('file', new Blob(['data'], { type: 'text/plain' }), 'a.txt');
  const parsed = await new Response(form).formData();
  out.push(`${parsed.get('name')} ${parsed.get('file').name} ${await parsed.get('file').text()}`);
  const headers = new Headers({ B: '2', a: '1' });
  headers.append('a', '3');
  out.push([...headers].map(([name, value]) => `${name}=${value}`).join('&'));
  out.push(await (await fetch('data:text/plain;base64,aGk=')).text());
  const controller = new AbortController();
  controller.abort();
  try { await fetch(`${base}/json`, { signal: controller.signal }); } catch (error) { out.push(error.name); }
  const stream = new Response('streamed').body.getReader();
  const { value } = await stream.read();
  out.push(`${new TextDecoder().decode(value)} ${Response.json({ x: 1 }).headers.get('content-type')}`);
  try { await fetch('http://127.0.0.1:1/'); } catch (error) { out.push(`${error.message} ${error.cause instanceof Error}`); }
  console.log(out.join('\\n'));
  server.close();
});",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    // Node 22 prints the same for this program.
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "200 true application/json a=1;b=2 {\"ok\":true}\nPOST /echo text/plain;charset=UTF-8 hello\ntrue true GET /echo - \ncompressed\nvalue a.txt data\na=1, 3&b=2\nhi\nAbortError\nstreamed application/json\nfetch failed true\n"
    );
}

#[test]
fn generated_app_uses_the_native_clip_filepaths_adapter() {
    // Only reads the clipboard; writing would replace the user's contents.
    let fixture = app(
        "import { helloWorld, readClipboardFilePaths, writeClipboardFilePaths } from 'clip-filepaths'; const content = readClipboardFilePaths(); let error = ''; try { writeClipboardFilePaths(['Z:/nativeboat-missing.png']); } catch (caught) { error = caught.message.split(':')[0]; } console.log(`${helloWorld()} ${Array.isArray(content.filePaths)} ${error}`);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "Hello from Rust 0.3.0 on Windows! true windows clipboard error\n"
    );
}

#[test]
fn generated_app_asserts_with_node_assert() {
    let fixture = app(
        "import assert from 'assert';\nconst out = [];\nconst code = (fn) => { try { fn(); return 'ok'; } catch (error) { return `${error.name}:${error.code}:${error.operator}`; } };\nout.push(code(() => assert.equal(1, '1')));\nout.push(code(() => assert.equal(1, 2)));\nout.push(code(() => assert.notEqual(1, 2)));\nout.push(code(() => assert.deepStrictEqual({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] })));\nout.push(code(() => assert.deepStrictEqual({ a: 1 }, { a: '1' })));\nout.push(code(() => assert.deepEqual({ a: 1 }, { a: '1' })));\nout.push(code(() => assert.strict.equal(1, '1')));\nout.push(code(() => assert.throws(() => { throw new TypeError('bad thing'); }, TypeError)));\nout.push(code(() => assert.throws(() => { throw new Error('bad thing'); }, /bad/)));\nout.push(code(() => assert.throws(() => {})));\nout.push(code(() => assert.ok(0)));\nout.push(code(() => assert.ifError(null)));\nout.push(code(() => assert.match('abc', /b/)));\nout.push(code(() => assert.fail('boom')));\nlet message = '';\ntry { assert.strictEqual(1, 2); } catch (error) { message = JSON.stringify(error.message); }\nout.push(message);\nassert.rejects(Promise.reject(new Error('x')), /x/).then(() => {\n  out.push('rejects ok');\n  console.log(out.join('\\n'));\n});",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    // Node 22 prints the same for this program.
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "ok\nAssertionError:ERR_ASSERTION:==\nok\nok\nAssertionError:ERR_ASSERTION:deepStrictEqual\nok\nAssertionError:ERR_ASSERTION:strictEqual\nok\nok\nAssertionError:ERR_ASSERTION:throws\nAssertionError:ERR_ASSERTION:==\nok\nok\nAssertionError:ERR_ASSERTION:fail\n\"Expected values to be strictly equal:\\n\\n1 !== 2\\n\"\nrejects ok\n"
    );
}

#[test]
fn generated_electron_menu_module_builds_and_retains_native_menu_trees() {
    let fixture = app(
        "import { BrowserWindow, Menu, MenuItem } from 'electron'; const menu = Menu.buildFromTemplate([{ label: 'File', submenu: [{ id: 'open', label: 'Open', enabled: false }, { type: 'separator' }] }]); const open = menu.getMenuItemById('open'); if (!open || open.enabled !== false || menu.items[0].submenu.items.length !== 2) throw 'menu template mismatch'; Menu.setApplicationMenu(menu); if (Menu.getApplicationMenu() !== menu) throw 'menu retention mismatch'; const context = new Menu(); context.append(new MenuItem({ id: 'copy', label: 'Copy' })); if (context.getMenuItemById('copy').label !== 'Copy') throw 'context menu mismatch'; const window = new BrowserWindow(); window.setMenu(context); window.removeMenu(); window.destroy(); console.log('native-menu-ready');",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"native-menu-ready\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn respects_module_format_without_silently_changing_script_semantics() {
    let fixture = app(
        "import value from './dependency.cjs'; if (value !== 'cjs') throw 'wrong CommonJS value';",
    );
    fixture.write(
        "dependency.cjs",
        b"if (this !== module.exports) throw 'changed CommonJS this'; module.exports = 'cjs';",
    );
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(!module_source(
        &fixture.path("dependency.cts"),
        "export const a = 1"
    ));
    assert!(!module_source(&fixture.path("dependency.CJS"), "var a = 1"));
    assert!(module_source(&fixture.path("dependency.mjs"), "let a = 1"));
    fixture.write("script/package.json", br#"{"type":"commonjs"}"#);
    assert!(!module_source(
        &fixture.path("script/dependency.js"),
        "let a = 1"
    ));
    assert!(!module_source(
        &fixture.path("script/dependency.ts"),
        "let a = 1"
    ));
    assert!(module_source(
        &fixture.path("script/dependency.ts"),
        "import type {Thing} from './types'; const a: Thing = 1;"
    ));
    fixture.write("nested/package.json", br#"{"type":"module"}"#);
    assert!(module_source(
        &fixture.path("nested/dependency.js"),
        "throw 'plain ES module'"
    ));
    fixture.write("nested/package.json", b"invalid package");
    assert!(!module_source(
        &fixture.path("nested/dependency.js"),
        "let a = 1"
    ));
    assert!(!module_source(Path::new("no-scope.js"), "let a = 1"));
    assert!(module_source(
        &fixture.path("dependency.jsx"),
        "export const a = 1"
    ));
}

#[test]
fn forwards_source_read_and_parse_errors_and_resolves_aliases_before_builtins() {
    let fixture = app("import {value} from 'electron'; if (value !== 2) throw 'bad alias';");
    fixture.write("replacement.ts", b"export const value = 2;");
    fixture.write(
        "tsconfig.json",
        br#"{"compilerOptions":{"baseUrl":".","paths":{"electron":["replacement.ts"]}}}"#,
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    assert!(compile(&project).errors.is_empty());
    fixture.write("replacement.ts", &[0xff]);
    assert!(
        compile(&project)
            .errors
            .join("\n")
            .contains("cannot read module 'electron'")
    );
    fixture.write("replacement.ts", b"export const = ;");
    let errors = compile(&project).errors.join("\n");
    assert!(errors.contains("replacement.ts:1:"), "{errors}");
    fixture.write("main.ts", b"process.stderr.cursorTo;");
    let errors = compile(&project).errors.join("\n");
    assert!(errors.contains("process.stderr.cursorTo"), "{errors}");
}

#[test]
fn resolves_relative_tool_paths_before_changing_to_staging_directory() {
    let fixture = app("export const value = 1;");
    let current = nativeboat_bundler::resolve::realpath(&std::env::current_exe().unwrap());
    let workspace =
        nativeboat_bundler::resolve::realpath(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let relative = Path::new("../..").join(current.strip_prefix(&workspace).unwrap());
    let output = invoke(&relative, &["--list".into()], fixture.root()).unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("native::tests::")
    );
}

#[test]
fn rejects_all_uncompiled_entries_and_configuration() {
    let fixture = app("export const value = 1;");
    fixture.write("preload.ts", b"import 'missing-preload';");
    fixture.write(
        "index.html",
        b"<h1>UI is not yet compiled</h1><script type=module src=/renderer.ts></script>",
    );
    fixture.write("renderer.ts", b"import 'missing-renderer';");
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    project.preloads.push(nativeboat_project::Entry {
        source: fixture.path("preload.ts"),
        runtime_paths: vec![],
    });
    project
        .defines
        .push(("ENV".into(), "process.env.ENV".into()));
    project.build_steps.push(nativeboat_project::scripts::Step {
        script: "must not run".into(),
        command: "echo bad > touched.txt".into(),
        optional: false,
    });
    let errors = compile(&project).errors.join("\n");
    assert!(errors.contains("missing-preload"));
    assert!(!errors.contains("native document UI for this renderer page"));
    assert!(
        errors.contains("renderer.ts:1:8: cannot resolve \"missing-renderer\""),
        "{errors}"
    );
    assert!(errors.contains("define 'ENV' is not a supported static literal"));
    // Compiling runs no packaging step.
    assert!(!fixture.path("touched.txt").exists());
    // A supported preload cannot be silently treated as main-process code.
    fixture.write("preload.ts", b"export const value = 2;");
    assert!(
        compile(&project)
            .errors
            .iter()
            .all(|error| { !error.contains("preload-to-renderer context bridge lifecycle") })
    );
    // Independently cover the remaining rejected configuration families.
    project.defines.clear();
    project
        .main_env
        .insert("NODE_ENV".into(), serde_json::json!("production"));
    project.build_steps.clear();
    assert!(!compile(&project).errors.is_empty());
    project.main_env.clear();
    project
        .extra_files
        .push(nativeboat_project::builder::ExtraFile {
            source: fixture.path("resource"),
            destination: "resource".into(),
        });
    assert!(!compile(&project).errors.is_empty());
}

#[test]
fn renderer_script_discovery_resolves_local_entries_and_rejects_uncompiled_scripts() {
    let fixture = app("");
    fixture.write(
        "index.html",
        b"<script type=module src='/src/app.ts?mode=dev#entry'></script>\n<script type=text/javascript src='./legacy.js'></script>\n<script type=application/json>{}</script>\n<script type=text/plain src='/ignored.js'></script>\n<script type=module>boot()</script>\n<script type=module></script>\n<script type=module src='https://example.test/app.js'></script>\n<script type=module src='//cdn.example.test/app.js'></script>\n<script type=module src='/missing.ts'></script>",
    );
    fixture.write("src/app.ts", b"export const app = true;");
    fixture.write("legacy.js", b"globalThis.ready = true;");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let page = project.pages.first().unwrap();
    let (entries_with_errors, found_errors) = renderer_scripts(&project, page).unwrap();
    let errors = found_errors.join("\n");
    // The inline module becomes a virtual module beside the page; the
    // classic script shares its top level.
    let inline = fixture.path(".index.inline-script-4.js");
    assert_eq!(
        entries_with_errors,
        vec![
            fixture.path("src/app.ts"),
            fixture.path("legacy.js"),
            inline.clone()
        ]
    );
    let key = nativeboat_bundler::resolve::realpath;
    assert_eq!(
        crate::classic_script::source(&key(&inline)).as_deref(),
        Some("boot()")
    );
    assert_eq!(
        crate::classic_script::source(&key(&fixture.path("legacy.js"))).as_deref(),
        Some("globalThis.ready = true;")
    );
    assert_eq!(
        crate::classic_script::source(&key(&fixture.path("src/app.ts"))),
        None
    );
    assert!(
        errors.contains("remote renderer script 'https://example.test/app.js'"),
        "{errors}"
    );
    assert!(
        errors.contains("remote renderer script '//cdn.example.test/app.js'"),
        "{errors}"
    );
    assert!(
        errors.contains("cannot resolve renderer script '/missing.ts'"),
        "{errors}"
    );

    fixture.write(
        "index.html",
        b"<script type=module src='/src/app.ts?mode=dev#entry'></script><script src='./legacy.js'></script><script type=application/json>{}</script>",
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let (entries, errors) = renderer_scripts(&project, project.pages.first().unwrap()).unwrap();
    assert!(errors.is_empty());
    assert_eq!(
        entries,
        vec![fixture.path("src/app.ts"), fixture.path("legacy.js")]
    );
    let missing_page = nativeboat_project::Entry {
        source: fixture.path("missing.html"),
        runtime_paths: Vec::new(),
    };
    assert!(
        renderer_scripts(&project, &missing_page).unwrap_err()[0]
            .contains("cannot read renderer HTML")
    );

    // The aggregate compiler keeps renderer discovery failures alongside
    // JavaScript diagnostics and handles pages with no scripts explicitly.
    fixture.write("index.html", b"<main>no script entries</main>");
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    project.pages.push(missing_page);
    let compilation = compile(&project);
    assert!(
        !compilation
            .errors
            .iter()
            .any(|error| { error.contains("native document UI for this renderer page") })
    );
    assert!(
        compilation
            .errors
            .iter()
            .any(|error| { error.contains("cannot read renderer HTML") })
    );
}

#[test]
fn pages_without_scripts_get_a_renderer_with_the_web_prelude() {
    let fixture = app("export const main = true;");
    fixture.write("index.html", b"<main>static page</main>");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let compilation = compile_with(
        &project,
        true,
        &crate::vue_tool::Components::new(),
        &crate::tailwind_tool::Sheets::new(),
    );
    assert!(compilation.errors.is_empty(), "{:?}", compilation.errors);
    assert_eq!(compilation.renderers.len(), 1);
    assert_eq!(compilation.renderers[0].entry_count, 1);
    // Without the prelude, such a page has nothing to run.
    assert!(compile(&project).renderers.is_empty());
}

#[test]
fn renderer_host_lowers_static_body_markup_into_native_node_specs() {
    let fixture = app("export const main = true;");
    fixture.write(
        "index.html",
        b"<html><body><div id='app' class='editor'><span>ready</span><!--anchor--><svg viewBox='0 0 2 2'><path d='M0 0'></path></svg></div><script type=module src='/renderer.ts'></script></body></html>",
    );
    fixture.write("renderer.ts", b"console.log('page');");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let compilation = compile(&project);
    assert_eq!(compilation.renderers.len(), 1);
    assert!(
        !compilation
            .errors
            .iter()
            .any(|error| { error.contains("native document UI for this renderer page") })
    );
    let (declarations, opener, after) = renderer_host_source(&project, &compilation, false, false);
    let host = format!("{declarations}{opener}{after}");
    assert!(host.contains("dom_binding::install_for_window(&mut *renderer_realm"));
    assert!(host.contains("NativeDocument, ::alloc::vec::Vec<::alloc::string::String>)>"));
    assert!(host.contains("let native_stylesheets = renderer_realm.take_native_stylesheets()"));
    assert!(host.contains("_nativeboat_renderer_realms.push((window, renderer_realm, native_document, native_stylesheets))"));
    assert!(host.contains("NodeSpec::Element { name: \"div\""));
    assert!(host.contains("(\"id\".into(), \"app\".into())"));
    assert!(host.contains("NodeSpec::Text(\"ready\".into())"));
    assert!(host.contains("NodeSpec::Comment(\"anchor\".into())"));
    assert!(host.contains("namespace: \"http://www.w3.org/2000/svg\".into()"));
    assert!(host.contains("(\"viewBox\".into(), \"0 0 2 2\".into())"));

    let (declarations, opener, after) = renderer_host_source(&project, &compilation, false, true);
    let native_host = format!("{declarations}{opener}{after}");
    assert!(native_host.contains("::nativeboat_win32::GdiMeasurer::new()"));
    // Pages with the Web APIs paint their live document's scene.
    assert!(native_host.contains("::nativeboat_page::DocumentPage::new"));
    assert!(native_host.contains("renderer_realm.install_web("));
    assert!(native_host.contains("if let Some(scene) = page.scene()"));
    assert!(native_host.contains("dom_binding::install_for_window(&mut *renderer_realm"));
    assert!(native_host.contains("unsafe { ::nativeboat_win32::paint_tree(dc, &scene.tree, scale"));
    assert!(native_host.contains("::nativeboat_native::app_host::set_paint_handler"));
    // Their windows take the page's input.
    assert!(native_host.contains("::nativeboat_page::PageWindow::adopt(page_view"));
    assert!(native_host.contains("_nativeboat_page_handler.add(page_window.clone())"));
}

#[test]
fn renderer_module_chunks_preserve_ids_and_compile_dependencies_first() {
    let sources = vec![
        vec!["fn m0() {}".to_owned()],
        vec!["fn require_c1() { require_c2(); }".to_owned()],
        vec!["fn require_c2() { require_c1(); }".to_owned()],
        vec![String::new()],
        vec!["fn m4() {}".to_owned()],
    ];
    let units = ModuleUnits::of(&sources);
    // Chunk crates import the names compiled modules use, such as the
    // property caches of each site.
    let prelude = dynamic_graph_prelude();
    assert!(prelude.contains(nativeboat_compiler::GENERATED_IMPORTS));
    assert!(prelude.contains("LookupCache"));
    let (chunks, module_chunk) = module_chunk_map(&units, 8);
    assert_eq!(chunks.len(), 3);
    assert_eq!(module_chunk, vec![Some(0), Some(1), Some(1), None, Some(2)]);
    assert_eq!(chunks[1], vec![1, 2]);

    let dependencies = chunk_dependencies(&units, &module_chunk, chunks.len());
    assert!(dependencies[1].is_empty());
    assert_eq!(chunk_compile_order(&dependencies), vec![0, 1, 2]);
    let entry = linked_entry_source(
        "pub fn entries() { m0(realm); require_c1(realm); m4(realm); }",
        "nativeboat_renderer_0_dispatcher",
        &[0, 1, 2, 4],
        &[1, 2],
        2,
        &[],
        &Default::default(),
    );
    assert!(entry.contains("nativeboat_renderer_0_dispatcher::evaluate_es(0, realm)"));
    assert!(entry.contains("nativeboat_renderer_0_dispatcher::evaluate_commonjs(1, realm)"));
    assert!(entry.contains("nativeboat_renderer_0_dispatcher::evaluate_es(4, realm)"));
    assert!(!entry.contains("::m0()"));
    assert!(entry.contains("module_entries(realm, &[0, 1])"));

    let leaves = vec![
        ForwarderLibrary {
            crate_name: "nativeboat_renderer_chunk_0".to_owned(),
            path: PathBuf::from("chunk0.rlib"),
            modules: vec![0],
            commonjs_modules: Vec::new(),
            is_dispatcher: false,
            fingerprint: 10,
            dependency_dirs: Vec::new(),
        },
        ForwarderLibrary {
            crate_name: "nativeboat_renderer_chunk_1".to_owned(),
            path: PathBuf::from("chunk1.rlib"),
            modules: vec![1, 2],
            commonjs_modules: vec![2],
            is_dispatcher: false,
            fingerprint: 11,
            dependency_dirs: Vec::new(),
        },
    ];
    let leaf_dispatcher = module_dispatcher_source("renderer_leaf_dispatcher", &leaves);
    assert!(leaf_dispatcher.contains("use ::alloc::vec::Vec;"));
    assert!(leaf_dispatcher.contains("pub fn evaluate_es(module: usize"));
    assert!(leaf_dispatcher.contains("let _ = (&cells, &cache);"));
    assert!(leaf_dispatcher.contains("0 => nativeboat_renderer_chunk_0::m0"));
    assert!(leaf_dispatcher.contains("1 => nativeboat_renderer_chunk_1::m1"));
    assert!(leaf_dispatcher.contains("2 => nativeboat_renderer_chunk_1::m2"));
    assert!(leaf_dispatcher.contains("2 => nativeboat_renderer_chunk_1::require_c2"));
    assert!(!leaf_dispatcher.contains("pub use"));

    let parent = ForwarderLibrary {
        crate_name: "renderer_leaf_dispatcher".to_owned(),
        path: PathBuf::from("dispatcher.rlib"),
        modules: vec![0, 1, 2],
        commonjs_modules: vec![2],
        is_dispatcher: true,
        fingerprint: 12,
        dependency_dirs: Vec::new(),
    };
    let root_dispatcher = module_dispatcher_source("renderer_root_dispatcher", &[parent]);
    assert!(root_dispatcher.contains("renderer_leaf_dispatcher::evaluate_es(0"));
    assert!(root_dispatcher.contains("renderer_leaf_dispatcher::evaluate_commonjs(2"));
}

#[test]
fn a_module_in_parts_spreads_over_chunks_that_name_each_other() {
    let sources = vec![
        vec!["fn m0() { m1(); }".to_owned()],
        vec![
            "pub static P1_0_S0: u8 = 0;\npub fn P1_0_f0() {}".to_owned(),
            "pub fn P1_1_f1() { P1_0_f0(); let _ = \"P1_0_f0\"; }".to_owned(),
            "fn m1() { P1_1_f1(); P1_0_S0; }\nfn require_c1() {}".to_owned(),
        ],
    ];
    let units = ModuleUnits::of(&sources);
    assert_eq!(units.first, vec![0, 1]);
    assert_eq!(units.last, vec![0, 3]);
    assert_eq!(units.named("P1_1_f1()"), Some(2));
    assert_eq!(units.named("m1("), Some(3));
    for other in ["P9_0_S0", "P1_7_S0", "P1_0S0", "Px", "m1x", "m9(", "mx"] {
        assert_eq!(units.named(other), None, "{other}");
    }
    // Parts compile before the parts and modules using them.
    let (chunks, unit_chunk) = module_chunk_map(&units, 1);
    assert_eq!(chunks, vec![vec![1], vec![2], vec![3], vec![0]]);
    assert_eq!(unit_chunk, vec![Some(3), Some(0), Some(1), Some(2)]);
    let dependencies = chunk_dependencies(&units, &unit_chunk, chunks.len());
    let expected: [&[usize]; 4] = [&[], &[0], &[0, 1], &[2]];
    for (chunk, expected) in expected.iter().enumerate() {
        assert_eq!(
            dependencies[chunk].iter().copied().collect::<Vec<_>>(),
            *expected
        );
    }
    let names = ["c0", "c1", "c2", "c3"].map(str::to_owned);
    let qualify = |unit: usize| {
        qualify_chunk_references(&units, unit, unit_chunk[unit].unwrap(), &unit_chunk, &names)
    };
    assert_eq!(qualify(0), "pub fn m0() { c2::m1(); }");
    assert_eq!(
        qualify(2),
        "pub fn P1_1_f1() { c0::P1_0_f0(); let _ = \"P1_0_f0\"; }"
    );
    assert_eq!(
        qualify(3),
        "pub fn m1() { c1::P1_1_f1(); c0::P1_0_S0; }\npub fn require_c1() {}"
    );
    // A chunk holding every part names them unqualified.
    let (chunks, unit_chunk) = module_chunk_map(&units, 1 << 20);
    assert_eq!(chunks, vec![vec![1, 2, 3, 0]]);
    assert_eq!(
        qualify_chunk_references(&units, 3, 0, &unit_chunk, &names),
        "pub fn m1() { P1_1_f1(); P1_0_S0; }\npub fn require_c1() {}"
    );
}

#[test]
fn generated_library_fingerprints_invalidate_source_toolchain_and_dependencies() {
    // The runtime's content is in the toolchain key; the folders libraries
    // are cached in are not part of the key.
    let generated_dependencies = [41_u64, 72_u64];
    let fingerprint = |toolchain: u64, source: &str, dependencies: &[u64]| {
        generated_rlib_fingerprint(
            toolchain,
            "nativeboat_renderer_chunk_0",
            source,
            dependencies,
        )
    };
    let base = fingerprint(9, "pub fn value() -> usize { 1 }", &generated_dependencies);
    assert_eq!(
        base,
        fingerprint(9, "pub fn value() -> usize { 1 }", &generated_dependencies)
    );
    assert_ne!(
        base,
        fingerprint(10, "pub fn value() -> usize { 1 }", &generated_dependencies)
    );
    assert_ne!(
        base,
        fingerprint(9, "pub fn value() -> usize { 2 }", &generated_dependencies)
    );
    assert_ne!(
        base,
        fingerprint(9, "pub fn value() -> usize { 1 }", &[41, 73])
    );
}

#[test]
fn generated_library_cache_keys_invalidate_changed_runtime_artifacts() {
    assert_eq!(
        generated_toolchain_key(9, 41),
        generated_toolchain_key(9, 41)
    );
    assert_ne!(
        generated_toolchain_key(9, 41),
        generated_toolchain_key(9, 42)
    );
    assert_ne!(
        generated_toolchain_key(9, 41),
        generated_toolchain_key(10, 41)
    );
}

#[test]
fn renderer_graph_compiles_stylesheets_into_realm_registered_native_modules() {
    let fixture = app("export const main = true;");
    fixture.write(
        "renderer.ts",
        b"import './theme.css'; export const loaded = true;",
    );
    fixture.write("theme.css", b"body { color: red } .note { margin: 4px }");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let mut stylesheets = std::collections::BTreeMap::new();
    let renderer = graph_entries_with_stylesheets(
        &project,
        &[fixture.path("renderer.ts")],
        &project.renderer_aliases,
        Target::RendererImport,
        false,
        &CompileOptions::default(),
        &mut stylesheets,
        &crate::vue_tool::Components::new(),
        &crate::tailwind_tool::Sheets::new(),
    )
    .unwrap();
    let identifier = format!(
        "nativeboat:stylesheet:{}",
        fixture.path("theme.css").display()
    );
    assert_eq!(
        stylesheets.get(&identifier).unwrap(),
        "body { color: red } .note { margin: 4px }"
    );
    assert!(renderer.native_dependencies().contains(&identifier));

    let source = context_rust_source(&NativeCompilation {
        main: None,
        preloads: Vec::new(),
        forks: Vec::new(),
        renderers: vec![CompiledContext {
            entry: fixture.path("renderer.ts"),
            pages: Vec::new(),
            entry_count: 1,
            module: renderer,
        }],
        stylesheets,
        errors: Vec::new(),
    });
    assert!(source.contains("register_native_stylesheet"));
    assert!(source.contains("body { color: red } .note { margin: 4px }"));
    assert_eq!(
        script_registration(&[("node_modules/pkg/main.js".to_owned(), 1)]),
        r#"realm.register_scripts(&[("node_modules/pkg/main.js", 1)]); "#
    );
}

#[test]
fn generated_executable_runs_a_compiled_stylesheet_module_without_an_engine() {
    let fixture = app("import './theme.css'; console.log('stylesheet-ready');");
    fixture.write("theme.css", b"body { color: rgb(1, 2, 3) }");
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"stylesheet-ready\n");
    assert!(executed.stderr.is_empty());
    assert!(
        imported_dlls(&executable).iter().all(|name| {
            !name.contains("boa") && !name.contains("v8") && !name.contains("node")
        })
    );
}

#[test]
fn stylesheet_imports_from_other_hosts_are_build_diagnostics() {
    let fixture = app("import './theme.css';");
    fixture.write(
        "theme.css",
        b"@import 'https://example.com/x.css';\nbody { color: red }\n@import './nested.css';",
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let errors = graph(
        &project,
        &project.main.source,
        &project.aliases,
        Target::MainImport,
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|error| {
            error.contains(&format!(
                "{}:1:1: stylesheets from other hosts cannot be imported",
                fixture.path("theme.css").display()
            ))
        }),
        "{errors:?}"
    );
    // The import after a rule is ignored, as browsers ignore it.
    assert!(
        !errors.iter().any(|error| error.contains(":3:1:")),
        "{errors:?}"
    );
}

#[test]
fn vue_script_setup_imports_are_discovered_with_original_component_locations() {
    let fixture = app("export const app = true;");
    fixture.write(
        "renderer.ts",
        b"import Component from './Component.vue'; void Component;",
    );
    fixture.write("support.ts", b"export const value = 3;");
    fixture.write(
        "Component.vue",
        b"<script setup lang=\"ts\">\nimport { value } from './support';\nclass Unsupported { accessor item = value; }\n</script>\n<template>\n  <div v-if=\"ready\">{{ value }}</div>\n</template>\n",
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children }); exports.toDisplayString = value => String(value); exports.createCommentVNode = () => ({ tag: '#comment' });",
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let errors = graph(
        &project,
        &fixture.path("renderer.ts"),
        &project.renderer_aliases,
        Target::RendererImport,
        &CompileOptions::default(),
    )
    .unwrap_err();
    let errors = errors.join("\n");
    assert!(errors.contains("Component.vue:3:"), "{errors}");
    assert!(
        !errors.contains("native Vue template compilation"),
        "{errors}"
    );
    assert!(!errors.contains("Unknown file extension"), "{errors}");
    assert!(
        !errors.contains("cannot resolve module './support'"),
        "{errors}"
    );
}

#[test]
fn renderer_graphs_use_components_compiled_by_the_project_compiler() {
    let fixture = app("export const app = true;");
    fixture.write(
        "renderer.ts",
        b"import App from './App.vue'; console.log(App.name);",
    );
    fixture.write("App.vue", b"<template><main/></template>");
    let mut components = crate::vue_tool::Components::new();
    components.insert(
        fixture.path("App.vue"),
        crate::vue_tool::Compiled {
            code: "export default { name: 'App' };".to_owned(),
            styles: vec!["main[data-v-1]{color:red}".to_owned()],
        },
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let mut stylesheets = std::collections::BTreeMap::new();
    graph_entries_with_stylesheets(
        &project,
        &[fixture.path("renderer.ts")],
        &project.renderer_aliases,
        Target::RendererImport,
        false,
        &CompileOptions::default(),
        &mut stylesheets,
        &components,
        &crate::tailwind_tool::Sheets::new(),
    )
    .unwrap();
    let style = format!(
        "nativeboat:stylesheet:{}.nativeboat-style-0.css",
        fixture.path("App.vue.nativeboat.ts").display()
    );
    assert_eq!(
        stylesheets.get(&style).map(String::as_str),
        Some("main[data-v-1]{color:red}"),
        "{stylesheets:?}"
    );
}

#[test]
fn compiles_static_vue_templates_into_native_render_behavior() {
    let fixture = app("import Component from './Separator.vue';\n\
         const vnode = Component.render();\n\
         console.log(vnode.tag, vnode.props.class, vnode.children[0].tag, vnode.children[0].children[0]);");
    fixture.write(
        "Separator.vue",
        b"<template><div class='pref-separator'><span>ready</span></div></template>",
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children });",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"div pref-separator span ready\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_script_setup_per_component_instance() {
    let fixture = app("import Component from './Component.vue';\n\
         console.log('imported');\n\
         const render = Component.setup({}, { expose() {} });\n\
         const vnode = render();\n\
         console.log(vnode.tag, vnode.children[0].children[0]);");
    fixture.write(
        "Component.vue",
        b"<script setup>\nconsole.log('setup-run')\nconst message = 'ready'\n</script>\n<template><div class='setup'><span>{{ message }}</span></div></template>",
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children }); exports.toDisplayString = value => String(value);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"imported\nsetup-run\ndiv ready\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_imported_vue_components_with_native_setup_props_and_slots() {
    let fixture = app("import Main from './Main.vue';\n\
         const render = Main.setup({}, { expose() {} });\n\
         const vnode = render();\n\
         console.log(vnode.tag.name, vnode.props.value, vnode.props.heading, vnode.children.head()[0].children[0], vnode.children.default()[0].children[0]);");
    fixture.write(
        "Main.vue",
        br#"<script setup>
import Compound from './compound.js'
const value = 'ready'
const heading = 'Options'
</script>
<template><Compound :value="value" v-bind:heading="heading"><template #head><span>{{ heading }}</span></template><button>open</button></Compound></template>"#,
    );
    fixture.write("compound.js", b"export default { name: 'Compound' };");
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children }); exports.toDisplayString = value => String(value);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"Compound ready Options Options open\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_vue_conditional_branches_in_render_closures() {
    let fixture = app("import Component from './Conditional.vue';\n\
         const render = Component.setup({}, { expose() {} });\n\
         const vnode = render();\n\
         console.log(vnode.children[0].tag, vnode.children[0].children[0]);");
    fixture.write(
        "Conditional.vue",
        br#"<script setup>
const state = 'second'
</script>
<template><div><span v-if="state === 'first'">first</span><i v-else-if="state === 'second'">second</i><b v-else>other</b></div></template>"#,
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children }); exports.createCommentVNode = () => ({ tag: '#comment' });",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"i second\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_vue_slot_outlets_with_named_default_and_fallback_content() {
    let fixture = app("import Parent from './Parent.vue';\n\
         const parent = Parent.setup({}, { expose() {} })();\n\
         const child = parent.tag.setup({}, { expose() {}, slots: parent.children })();\n\
         console.log(child.children[0].children[0].children[0], child.children[1].children[0].children[0]);");
    fixture.write(
        "Parent.vue",
        br#"<script setup>
import Child from './Child.vue'
</script>
<template><Child><template #heading><strong>Title</strong></template><em>Body</em></Child></template>"#,
    );
    fixture.write(
        "Child.vue",
        br#"<script setup></script>
<template><section><slot name="heading"><span>Fallback</span></slot><slot><i>Default fallback</i></slot></section></template>"#,
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children }); exports.renderSlot = (slots, name, props, fallback) => ({ tag: '#slot', children: slots[name] ? slots[name]() : fallback() });",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"Title Body\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_vue_v_for_iterations_into_fragment_children() {
    let fixture = app("import List from './List.vue';\n\
         const render = List.setup({}, { expose() {} });\n\
         const vnode = render();\n\
         console.log(vnode.children[0].children.length, vnode.children[0].children[0].children[0], vnode.children[0].children[0].children[2], vnode.children[0].children[1].children[2]);");
    fixture.write(
        "List.vue",
        br#"<script setup>
const items = ['a', 'b']
</script>
<template><ul><li v-for="(item, index) in items" :key="index">{{ index }}:{{ item }}</li></ul></template>"#,
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children }); exports.toDisplayString = value => String(value); exports.renderList = (source, render) => source.map(render); exports.Fragment = '#fragment';",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"2 0 a b\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_vue_event_handlers_and_stop_prevent_modifiers() {
    let fixture = app("import Button from './Button.vue';\n\
         const render = Button.setup({}, { expose() {} });\n\
         const vnode = render();\n\
         vnode.props.onClick({ stopPropagation() { console.log('stop'); }, preventDefault() { console.log('prevent'); } });");
    fixture.write(
        "Button.vue",
        br#"<script setup>
const activate = (event) => console.log(event.type)
</script>
<template><button @click.stop.prevent="activate">go</button></template>"#,
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children });",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"stop\nprevent\nundefined\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_global_component_v_model_updates_for_setup_refs() {
    let fixture = app("import Switch from './Switch.vue';\n\
         const render = Switch.setup({}, { expose() {} });\n\
         let vnode = render();\n\
         vnode.props['onUpdate:modelValue'](true);\n\
         vnode = render();\n\
         console.log(vnode.tag.name, vnode.props.modelValue);");
    fixture.write(
        "Switch.vue",
        br#"<script setup>
import { ref } from 'vue'
const checked = ref(false)
</script>
<template><el-switch v-model="checked" /></template>"#,
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.ref = value => ({ __v_isRef: true, value }); exports.h = (tag, props, children) => ({ tag, props, children }); exports.resolveComponent = name => ({ name });",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"el-switch true\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_v_show_refs_dynamic_component_tags_and_bound_object_spreads() {
    let fixture = app("import View from './View.vue';\n\
         const render = View.setup({}, { expose() {} });\n\
         const vnode = render();\n\
         console.log(vnode.tag.name, vnode.props.title, vnode.directives[0][0], vnode.directives[0][1]);\n\
         vnode.props.ref({ name: 'mounted' });");
    fixture.write(
        "View.vue",
        br#"<script setup>
import { ref } from 'vue'
const control = ref({ name: 'Control' })
const attrs = { title: 'spread' }
const visible = ref(true)
const input = ref(null)
</script>
<template><component :is="control" v-bind="attrs" v-show="visible" ref="input" /></template>"#,
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.ref = value => ({ __v_isRef: true, value }); exports.h = (tag, props, children) => ({ tag, props, children }); exports.withDirectives = (vnode, directives) => Object.assign(vnode, { directives }); exports.vShow = 'show';",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"Control spread show true\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_script_setup_props_defaults_emits_and_exposed_methods() {
    let fixture = app("import View from './View.vue';\n\
         let exposed;\n\
         const render = View.setup({ title: undefined }, { expose(value) { exposed = value; }, emit(event, value) { return event + ':' + value; }, slots: {} });\n\
         const vnode = render();\n\
         console.log(vnode.children[0], exposed.title, exposed.fire('saved'));");
    fixture.write(
        "View.vue",
        br#"<script setup lang="ts">
const props = withDefaults(defineProps<{ title?: string }>(), { title: 'fallback' })
const emit = defineEmits<{ (event: 'change', value: string): void }>()
defineExpose({ title: props.title, fire: (value: string) => emit('change', value) })
</script>
<template><span>{{ props.title }}</span></template>"#,
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children }); exports.toDisplayString = value => String(value);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"fallback fallback change:saved\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_static_svg_imports_as_vue_components() {
    let fixture = app("import Icon from './icon.svg';\n\
         const vnode = Icon.render();\n\
         console.log(vnode.tag, vnode.props.viewBox, vnode.children[0].tag, vnode.children[0].props.d);");
    fixture.write(
        "icon.svg",
        b"<svg viewBox='0 0 20 20'><path d='M0 0' /></svg>",
    );
    fixture.write("node_modules/vue/package.json", b"{\"main\":\"index.js\"}");
    fixture.write(
        "node_modules/vue/index.js",
        b"exports.h = (tag, props, children) => ({ tag, props, children });",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(executed.stdout, b"svg 0 0 20 20 path M0 0\n");
    assert!(executed.stderr.is_empty());
}

#[test]
fn compiles_renderer_browser_false_mappings_as_empty_modules() {
    let fixture = app("export const main = true;");
    fixture.write(
        "renderer.ts",
        b"import inspect from 'object-inspect'; export const value = inspect;",
    );
    fixture.write(
        "node_modules/object-inspect/package.json",
        br#"{"main":"index.js","browser":{"./util.inspect.js":false}}"#,
    );
    fixture.write(
        "node_modules/object-inspect/index.js",
        b"module.exports = require('./util.inspect');",
    );
    fixture.write(
        "node_modules/object-inspect/util.inspect.js",
        b"throw new Error('browser false mapping was ignored');",
    );
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let graph = graph(
        &project,
        &fixture.path("renderer.ts"),
        &project.aliases,
        Target::RendererImport,
        &CompileOptions::default(),
    )
    .unwrap();
    assert!(graph.rust_source().contains("browser-empty-module.cjs"));
}

#[test]
fn reports_unsupported_svg_component_markup() {
    let fixture = app("import Icon from './icon.svg'; console.log(Icon);");
    fixture.write("icon.svg", b"<svg><script>alert(1)</script></svg>");
    let (code, _, err) = run(&fixture, &["--out", "native-output"]);
    assert_ne!(code, 0);
    assert!(
        err.contains(
            "icon.svg:1:1: SVG component imports must contain supported static SVG markup"
        ),
        "{err}"
    );
}

#[test]
fn renderer_page_scripts_share_one_module_graph_and_dependency_cache() {
    let fixture = app("");
    fixture.write(
        "first.ts",
        b"import { value } from './shared.ts'; export const first = value;",
    );
    fixture.write(
        "second.ts",
        b"import { value } from './shared.ts'; export const second = value;",
    );
    fixture.write("shared.ts", b"export const value = 42;");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let module = graph_entries(
        &project,
        &[fixture.path("first.ts"), fixture.path("second.ts")],
        &project.renderer_aliases,
        Target::RendererImport,
        &CompileOptions::default(),
    )
    .unwrap();

    assert_eq!(module.entry_count(), 2);
    // Both page entries import the same source module, so one page graph has
    // exactly three module evaluators: the shared dependency and two entries.
    assert_eq!(module.rust_source().matches("fn evaluate_").count(), 3);
    assert_eq!(
        graph_entries(
            &project,
            &[],
            &project.renderer_aliases,
            Target::RendererImport,
            &CompileOptions::default(),
        )
        .unwrap_err(),
        ["an execution context must have an entry"]
    );
}

#[test]
fn compilation_retains_successful_preload_and_renderer_context_graphs() {
    let fixture = app("export const main = true;");
    fixture.write("preload.ts", b"export const preload = true;");
    fixture.write(
        "index.html",
        b"<script type=module src='/renderer.ts'></script><script type=module src='/second.ts'></script>",
    );
    fixture.write(
        "renderer.ts",
        b"import './shared.ts'; export const renderer = true;",
    );
    fixture.write(
        "second.ts",
        b"import './shared.ts'; export const second = true;",
    );
    fixture.write("shared.ts", b"export const shared = true;");
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    project.preloads.push(nativeboat_project::Entry {
        source: fixture.path("preload.ts"),
        runtime_paths: Vec::new(),
    });
    project.pages.clear();
    project.pages.push(nativeboat_project::Entry {
        source: fixture.path("index.html"),
        runtime_paths: Vec::new(),
    });
    project.renderer_root = fixture.root().to_path_buf();

    let compilation = compile(&project);
    assert!(compilation.main.is_some());
    assert_eq!(compilation.preloads.len(), 1);
    assert_eq!(compilation.renderers.len(), 1);
    assert_eq!(compilation.modules().count(), 3);
    assert!(
        compilation
            .errors
            .iter()
            .all(|error| { !error.contains("preload-to-renderer context bridge lifecycle") })
    );
    assert!(compilation.errors.is_empty(), "{:?}", compilation.errors);

    let contexts = context_rust_source(&compilation);
    assert!(contexts.contains("mod preload_0"));
    assert!(contexts.contains("mod renderer_0"));
    assert!(contexts.contains("preload.ts"));
    assert!(contexts.contains("index.html"));
    assert!(
        contexts.contains("pub fn initialize(realm: &mut Realm) -> Result<Vec<Object>, Thrown> { module_entries(realm, &[0, 1]) }")
    );
    let source = application_source(&project, &compilation, false, false).unwrap();
    assert!(source.contains("extern crate nativeboat_preload_0 as preload_0"));
    assert!(source.contains("extern crate nativeboat_renderer_0 as renderer_0"));
    assert!(source.contains("extern crate nativeboat_main_0 as nativeboat_main_graph"));
    assert!(source.contains("realm.run_main(::nativeboat_main_graph::initialize)"));
    let preload = context_library_source(
        "preload",
        0,
        &compilation.preloads[0],
        &compilation.stylesheets,
    );
    assert!(preload.starts_with("#![no_std]\nextern crate alloc;"));
    assert!(preload.contains("pub fn initialize(realm: &mut Realm)"));
    assert!(preload.contains("module_entries(realm, &[0])"));
    let node_source = application_source(&project, &compilation, true, false).unwrap();
    assert!(node_source.contains("realm.node_library = Some(::nativeboat_node::require);"));
}

/// A mocked rustc run that succeeds and, like rustc, writes its `-o` output.
fn compiled(_: &Path, args: &[OsString], directory: &Path) -> std::io::Result<Output> {
    if let Some(index) = args.iter().position(|arg| arg == "-o") {
        let output = directory.join(&args[index + 1]);
        std::fs::create_dir_all(output.parent().unwrap())?;
        std::fs::write(output, b"library")?;
    }
    Ok(success())
}

#[test]
fn failed_native_toolchains_preserve_existing_output_and_clean_staging() {
    let fixture = app("export const value = 1;");
    fixture.write("dist/native-test.exe", b"previous executable");
    let (code, _, err) = run(&fixture, &["--rustc", "missing-nativeboat-rustc"]);
    assert_eq!(code, 1);
    assert!(err.contains("cannot run missing-nativeboat-rustc"), "{err}");
    assert_eq!(
        std::fs::read(fixture.path("dist/native-test.exe")).unwrap(),
        b"previous executable"
    );
    assert_eq!(std::fs::read_dir(fixture.path("dist")).unwrap().count(), 1);
    {
        let mut session = shimforge::Session::new();
        let invoke_mock = shimforge::mock!(
            session,
            invoke,
            fn(&Path, &[OsString], &Path) -> std::io::Result<Output>
        );
        invoke_mock.expect().once().returning(|_, _, _| {
            Ok(Output {
                status: std::process::ExitStatus::from_raw(3),
                stdout: b"compiler output".to_vec(),
                stderr: b"compiler failure".to_vec(),
            })
        });
        let (_, _, err) = run(&fixture, &[]);
        assert!(err.contains("native toolchain failed"), "{err}");
        assert!(err.contains("compiler outputcompiler failure"));
        assert_eq!(std::fs::read_dir(fixture.path("dist")).unwrap().count(), 1);
    }
    {
        let mut session = shimforge::Session::new();
        let tool = shimforge::mock!(
            session,
            invoke,
            fn(&Path, &[OsString], &Path) -> std::io::Result<Output>
        );
        tool.expect().returning(|tool, args, directory| {
            if args[0] == "app.rs" {
                Ok(Output {
                    status: std::process::ExitStatus::from_raw(3),
                    stdout: Vec::new(),
                    stderr: b"app linking failed".to_vec(),
                })
            } else {
                compiled(tool, args, directory)
            }
        });
        let err = run(&fixture, &[]).2;
        assert!(err.contains("app linking failed"), "{err}");
        assert_eq!(
            std::fs::read(fixture.path("dist/native-test.exe")).unwrap(),
            b"previous executable"
        );
        assert_eq!(std::fs::read_dir(fixture.path("dist")).unwrap().count(), 1);
    }
}

#[test]
fn reports_output_staging_write_and_publication_failures() {
    let fixture = app("export const value = 1;");
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    let compilation = compile(&project);
    assert!(compilation.errors.is_empty());
    let args = settings(&fixture);
    let mut rename_attempts = 0;
    publish_with_retry(
        Path::new("staged.exe"),
        Path::new("published.exe"),
        |_, _| {
            rename_attempts += 1;
            if rename_attempts < 3 {
                Err(std::io::ErrorKind::PermissionDenied.into())
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    assert_eq!(rename_attempts, 3);
    let staged = fixture.path("staged.exe");
    let published = fixture.path("published.exe");
    std::fs::write(&staged, b"native executable").unwrap();
    publish_with_retry(&staged, &published, rename_executable).unwrap();
    assert!(!staged.exists());
    assert_eq!(std::fs::read(published).unwrap(), b"native executable");
    let mut denied_attempts = 0;
    assert_eq!(
        publish_with_retry(
            Path::new("staged.exe"),
            Path::new("published.exe"),
            |_, _| {
                denied_attempts += 1;
                Err(std::io::ErrorKind::PermissionDenied.into())
            }
        )
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(denied_attempts, 7);
    let missing_main = NativeCompilation {
        main: None,
        preloads: Vec::new(),
        forks: Vec::new(),
        renderers: Vec::new(),
        stylesheets: std::collections::BTreeMap::new(),
        errors: Vec::new(),
    };
    assert_eq!(
        link_inner(&project, &missing_main, &output(&project, &args), &args).unwrap_err(),
        "main module did not compile"
    );
    for name in ["", ".", "..", "../escaped", "nested\\name", "bad:name"] {
        project.metadata.product_name = name.into();
        assert!(
            link(&project, &compilation, &output(&project, &args), &args).unwrap_err()[0]
                .contains("filename")
        );
    }
    project.metadata.product_name = "native-test".into();
    fixture.write("native-output", b"blocking file");
    assert!(
        link(&project, &compilation, &output(&project, &args), &args).unwrap_err()[0]
            .contains("cannot create native output")
    );
    std::fs::remove_file(fixture.path("native-output")).unwrap();
    {
        let mut session = shimforge::Session::new();
        let create = shimforge::mock!(session, create_directory, fn(&Path) -> std::io::Result<()>);
        // Every staging name is taken; the first is a killed build's.
        let first = std::cell::Cell::new(true);
        create.expect().returning(move |path| {
            if first.replace(false) {
                std::fs::create_dir_all(path).unwrap();
                std::fs::write(path.join("previous-build.txt"), "must survive").unwrap();
            }
            Err(std::io::ErrorKind::AlreadyExists.into())
        });
        assert!(
            link(&project, &compilation, &output(&project, &args), &args).unwrap_err()[0]
                .contains("staging directory")
        );
    }
    {
        let mut session = shimforge::Session::new();
        let write = shimforge::mock!(session, write_file, fn(&Path, &[u8]) -> std::io::Result<()>);
        write
            .expect()
            .returning(|_, _| Err(std::io::Error::other("disk full")));
        assert!(
            link(&project, &compilation, &output(&project, &args), &args).unwrap_err()[0]
                .contains("cannot write")
        );
    }
    {
        let mut session = shimforge::Session::new();
        let tool = shimforge::mock!(
            session,
            invoke,
            fn(&Path, &[OsString], &Path) -> std::io::Result<Output>
        );
        tool.expect().returning(compiled);
        let write = shimforge::mock!(session, write_file, fn(&Path, &[u8]) -> std::io::Result<()>);
        write.expect().returning(|path, content| {
            if path.file_name().unwrap() == "app.rs" {
                Err(std::io::Error::other("app source full"))
            } else {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(path, content)
            }
        });
        assert!(
            link(&project, &compilation, &output(&project, &args), &args).unwrap_err()[0]
                .contains("generated native application")
        );
    }
    {
        let mut session = shimforge::Session::new();
        let tool = shimforge::mock!(
            session,
            invoke,
            fn(&Path, &[OsString], &Path) -> std::io::Result<Output>
        );
        // The libraries compile, but no executable is written to publish.
        tool.expect().returning(|tool, args, directory| {
            if args[0] == "app.rs" {
                Ok(success())
            } else {
                compiled(tool, args, directory)
            }
        });
        assert!(
            link(&project, &compilation, &output(&project, &args), &args).unwrap_err()[0]
                .contains("cannot publish")
        );
    }
    // Besides the application's files, only the earlier build is there.
    let entries: Vec<_> = std::fs::read_dir(fixture.path("native-output"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().unwrap() != "resources")
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    let preserved = entries[0].clone();
    assert_eq!(
        std::fs::read_to_string(preserved.join("previous-build.txt")).unwrap(),
        "must survive"
    );
}

#[test]
fn packaged_toolchains_supply_the_runtime_libraries_precompiled() {
    let fixture = Fixture::new(&[
        ("nativeboat.exe", ""),
        ("toolchain/bin/rustc.exe", ""),
        (
            "toolchain/runtime-libs/matcher/libnativeboat_regexp.rlib",
            "",
        ),
        (
            "toolchain/runtime-libs/runtime-web/libnativeboat_start.rlib",
            "",
        ),
    ]);
    let bundled = select_toolchain(None, &fixture.path("nativeboat.exe"));
    assert_eq!(
        shipped(&bundled, "matcher", "libnativeboat_regexp.rlib"),
        Some(fixture.path("toolchain/runtime-libs/matcher"))
    );
    // A folder without its library, or a missing kind, is not used.
    assert_eq!(
        shipped(&bundled, "runtime-web", "libnativeboat_native.rlib"),
        None
    );
    assert_eq!(shipped(&bundled, "page", "libnativeboat_page.rlib"), None);
    let explicit = select_toolchain(
        Some(Path::new("custom-rustc")),
        &fixture.path("nativeboat.exe"),
    );
    assert_eq!(
        shipped(&explicit, "matcher", "libnativeboat_regexp.rlib"),
        None
    );
    // Development builds and unit tests compile their own.
    assert_eq!(
        prebuilt(&bundled, "matcher", "libnativeboat_regexp.rlib"),
        None
    );
}

#[test]
fn builds_use_an_explicit_compiler_then_the_bundled_toolchain_then_the_search_path() {
    let fixture = Fixture::new(&[("nativeboat.exe", ""), ("toolchain/bin/rustc.exe", "")]);
    let program = fixture.path("nativeboat.exe");
    let bundled = select_toolchain(None, &program);
    assert_eq!(bundled.rustc, fixture.path("toolchain/bin/rustc.exe"));
    assert_eq!(bundled.sysroot, Some(fixture.path("toolchain")));
    let explicit = select_toolchain(Some(Path::new("custom-rustc")), &program);
    assert_eq!(explicit.rustc, Path::new("custom-rustc"));
    assert!(explicit.sysroot.is_none());
    let development = select_toolchain(None, &fixture.path("elsewhere/nativeboat.exe"));
    assert_eq!(development.rustc, Path::new("rustc"));
    assert!(development.sysroot.is_none());
    // The bundled sysroot precedes every compiler argument.
    let sysroot = fixture.path("toolchain").into_os_string();
    let mut session = shimforge::Session::new();
    let invoke_mock = shimforge::mock!(
        session,
        invoke,
        fn(&Path, &[OsString], &Path) -> std::io::Result<Output>
    );
    invoke_mock.expect().once().returning(move |_, args, _| {
        assert_eq!(
            args,
            [
                OsString::from("--sysroot"),
                sysroot.clone(),
                "app.rs".into()
            ]
        );
        Ok(success())
    });
    toolchain(&bundled, &["app.rs".into()], fixture.root()).unwrap();
}

/// The data of one resource of an executable, read by Windows itself.
fn resource(executable: &Path, kind: u16, name: u16) -> Option<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        FindResourceW, LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW,
        LoadResource, LockResource, SizeofResource,
    };
    let path: Vec<u16> = executable.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: the module is loaded as data only, read, and released.
    unsafe {
        let module = LoadLibraryExW(
            path.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        );
        assert!(!module.is_null());
        let found = FindResourceW(
            module,
            name as usize as *const u16,
            kind as usize as *const u16,
        );
        let data = (!found.is_null()).then(|| {
            let size = SizeofResource(module, found) as usize;
            let bytes = LockResource(LoadResource(module, found)) as *const u8;
            std::slice::from_raw_parts(bytes, size).to_vec()
        });
        windows_sys::Win32::Foundation::FreeLibrary(module);
        data
    }
}

#[test]
fn executables_carry_the_manifest_version_information_and_icon() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"notes","productName":"Notes","version":"2.5.1","description":"Native notes","type":"module","main":"main.ts","devDependencies":{"electron":"44"},"build":{"win":{"icon":"build/icon.ico"}}}"#,
        ),
        ("main.ts", "export const ready = true;"),
    ]);
    let mut icon = Vec::new();
    for value in [0u16, 1, 1] {
        icon.extend_from_slice(&value.to_le_bytes());
    }
    icon.extend_from_slice(&[16, 16, 0, 0, 1, 0, 32, 0]);
    icon.extend_from_slice(&4u32.to_le_bytes());
    icon.extend_from_slice(&22u32.to_le_bytes());
    icon.extend_from_slice(b"icon");
    fixture.write("build/icon.ico", &icon);
    let (code, _, err) = run(&fixture, &[]);
    assert_eq!(code, 0, "{err}");
    let executable = fixture.path("dist/Notes.exe");
    let manifest = String::from_utf8(resource(&executable, 24, 1).unwrap()).unwrap();
    assert!(manifest.contains("PerMonitorV2"));
    assert!(manifest.contains("Microsoft.Windows.Common-Controls"));
    let version = resource(&executable, 16, 1).unwrap();
    let text: Vec<u8> = "Native notes"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert!(version.windows(text.len()).any(|window| window == text));
    assert_eq!(resource(&executable, 3, 1).unwrap(), b"icon");
    assert!(resource(&executable, 14, 1).is_some());
    assert!(Command::new(&executable).status().unwrap().success());

    // An unusable icon stops the build before anything is published.
    fixture.write("build/icon.ico", b"not an icon");
    std::fs::remove_file(&executable).unwrap();
    let (code, _, err) = run(&fixture, &[]);
    assert_eq!(code, 1);
    assert!(err.contains("the icon must be an .ico file"), "{err}");
    assert!(!executable.exists());

    // A resource file that cannot be written is reported.
    fixture.write("build/icon.ico", &icon);
    let mut session = shimforge::Session::new();
    let write = shimforge::mock!(session, write_file, fn(&Path, &[u8]) -> std::io::Result<()>);
    write.expect().returning(|path, content| {
        if path.file_name().unwrap() == "app.res" {
            Err(std::io::Error::other("resources full"))
        } else {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, content)
        }
    });
    let (code, _, err) = run(&fixture, &[]);
    assert_eq!(code, 1);
    assert!(err.contains("app.res: resources full"), "{err}");
}

#[test]
fn signing_commands_run_on_the_executable_before_it_is_published() {
    assert_eq!(
        signing_command("signtool sign /a {file}", Path::new("C:/out/app.exe")),
        "signtool sign /a \"C:/out/app.exe\""
    );
    assert_eq!(
        signing_command("sign-tool", Path::new("C:/out/app.exe")),
        "sign-tool \"C:/out/app.exe\""
    );
    let fixture = app("export const ready = true;");
    // The command runs from the project directory on the staged executable.
    let (code, _, err) = run(
        &fixture,
        &[
            "--sign",
            "echo SIGNED-BY-CUSTOMER>>{file} && echo ok> signed.txt",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let executable = fixture.path("dist/native-test.exe");
    let image = std::fs::read(&executable).unwrap();
    let mark = b"SIGNED-BY-CUSTOMER";
    assert!(image.windows(mark.len()).any(|window| window == mark));
    assert!(fixture.path("signed.txt").is_file());
    assert!(Command::new(&executable).status().unwrap().success());
    // A failed signature keeps the previous executable.
    let (code, _, err) = run(&fixture, &["--sign", "echo refused & exit /b 5"]);
    assert_eq!(code, 1);
    assert!(err.contains("signing failed"), "{err}");
    assert!(err.contains("refused"), "{err}");
    assert_eq!(std::fs::read(&executable).unwrap(), image);
    // A shell that cannot start is reported.
    let mut session = shimforge::Session::new();
    let shell = shimforge::mock!(session, crate::steps::shell, fn() -> PathBuf);
    shell
        .expect()
        .returning(|| PathBuf::from("missing-nativeboat-shell.exe"));
    let error = sign("sign", &executable, fixture.root()).unwrap_err();
    assert!(
        error.starts_with("cannot run the signing command"),
        "{error}"
    );
}

#[test]
fn built_timeouts_refresh_after_completion_and_preserve_handle_state() {
    // Recorded Node 22.15.0 output; all timer ordering is causally chained.
    let fixture = app(r#"let count = 0;
let conversions = 0;
try { setTimeout(7, { valueOf() { conversions++; return 1; } }); }
catch (error) { console.log(error.name, conversions); }
const cleared = setTimeout(() => console.log('never'), 100);
clearTimeout(cleared);
console.log('cancelled', cleared.hasRef());
cleared.unref(); console.log('unref', cleared.hasRef());
cleared.ref(); cleared.refresh();
const timer = setTimeout(function(value) {
  count++;
  console.log(value, count, this.hasRef());
  if (count === 1) {
    const old = Number(this);
    this.refresh();
    console.log('same', Number(this) === old);
  } else if (count === 2) {
    queueMicrotask(() => {
      const old = Number(timer);
      timer.refresh();
      console.log('new', Number(timer) !== old);
      clearTimeout(old);
    });
  } else {
    clearTimeout(this);
    this.refresh();
    console.log('finished', this.hasRef());
  }
}, 1.9, 'tick');
"#);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "TypeError 0\ncancelled true\nunref false\ntick 1 true\nsame true\ntick 2 true\nnew true\ntick 3 true\nfinished true\n"
    );
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_direct_native_process_identity_and_preserves_output_on_unsupported_process_use() {
    let fixture = app(
        "console.log(process.platform, process['arch'], process.pid); console.log(typeof process, process?.platform);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no JavaScript engine)"));
    let executable = fixture.path("native-output/native-test.exe");
    let child = Command::new(&executable)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let executed = child.wait_with_output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        format!("win32 x64 {pid}\nobject win32\n")
    );
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    let prior = std::fs::read(&executable).unwrap();
    {
        fixture.write("main.ts", b"process.abort();");
        let (code, _, err) = run(&fixture, &["--out", "native-output"]);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("main.ts:1:"), "{err}");
        assert!(err.contains("process"), "{err}");
        assert_eq!(std::fs::read(&executable).unwrap(), prior);
    }
}

#[test]
fn native_process_paths_follow_the_launched_executable_and_unicode_directory() {
    let fixture = app(
        "const cwd = process.cwd; console.log(cwd()); console.log(process.execPath); console.log(cwd === process['cwd'], cwd.call(null) === process?.cwd());",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let built = fixture.path("native-output/native-test.exe");
    let directory = fixture.path("工作目录 🛤");
    std::fs::create_dir(&directory).unwrap();
    let executable = directory.join("原生程序.exe");
    std::fs::copy(&built, &executable).unwrap();
    let executed = Command::new(&executable)
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(executed.stderr.is_empty());
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        format!(
            "{}\n{}\ntrue true\n",
            directory.display(),
            executable.display()
        )
    );
    let imports = imported_dlls(&executable);
    assert!(
        imports.iter().all(|name| {
            [
                "kernel32.dll",
                "bcryptprimitives.dll",
                "bcrypt.dll",
                "user32.dll",
                "winhttp.dll",
                "ws2_32.dll",
            ]
            .contains(&name.as_str())
        }),
        "unexpected native imports: {imports:?}"
    );
    {
        let source = "process.chdir('C:/'); console.log(process.cwd());";
        fixture.write("main.ts", source.as_bytes());
        let (code, out, err) = run(&fixture, &["--out", "native-output"]);
        assert_eq!(code, 0, "{out}{err}");
        let executed = Command::new(&built).output().unwrap();
        assert!(executed.status.success(), "{executed:?}");
        assert_eq!(
            String::from_utf8(executed.stdout).unwrap(),
            format!("C:\\{}", "\n")
        );
    }
}

#[test]
fn native_environment_uses_launch_values_and_keeps_parent_state_isolated() {
    let parent_value = std::env::var_os("NATIVEBOAT_ARTIFACT_ENV");
    let parent_empty = std::env::var_os("NATIVEBOAT_ARTIFACT_EMPTY");
    let fixture = app(
        "const env=process.env;const key='NATIVEBOAT_ARTIFACT_ENV';console.log(env[key],env===process['env']);env[key]='changed';console.log(env[key.toLowerCase()]);env[key]='';console.log(env[key]==='',key in env);delete env[key];console.log(env[key]===undefined);console.log(Object.keys(env).includes('NATIVEBOAT_ARTIFACT_EMPTY'),env.NATIVEBOAT_ARTIFACT_EMPTY==='');",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable)
        .env("NATIVEBOAT_ARTIFACT_ENV", "继承 🛤")
        .env("NATIVEBOAT_ARTIFACT_EMPTY", "")
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "继承 🛤 true\nchanged\ntrue true\ntrue\ntrue true\n"
    );
    assert!(executed.stderr.is_empty());
    assert_eq!(std::env::var_os("NATIVEBOAT_ARTIFACT_ENV"), parent_value);
    assert_eq!(std::env::var_os("NATIVEBOAT_ARTIFACT_EMPTY"), parent_empty);
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    let prior = std::fs::read(&executable).unwrap();
    {
        let source =
            "import {readFileSync} from 'node:missing-native-module'; console.log(readFileSync);";
        fixture.write("main.ts", source.as_bytes());
        let (code, _, err) = run(&fixture, &["--out", "native-output"]);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("main.ts:1:"), "{err}");
        assert_eq!(std::fs::read(&executable).unwrap(), prior);
    }
}

#[test]
fn native_arguments_follow_windows_launch_quoting_without_an_entry_script() {
    let fixture = app(
        "console.log(JSON.stringify(process.argv.slice(1)));console.log(process.argv[0]===process.execPath,process.argv===process['argv']);process.argv.push('mutated');console.log(process.argv[process.argv.length-1]);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let built = fixture.path("native-output/native-test.exe");
    let directory = fixture.path("Argument folder 测试");
    std::fs::create_dir(&directory).unwrap();
    let executable = directory.join("程序名字.exe");
    std::fs::copy(&built, &executable).unwrap();
    let arguments = [
        "",
        "two words",
        "tab\there",
        "quote\"here",
        r"尾部\",
        r#"slashes\\\"quote"#,
        "中 🛤",
        "^literal",
        "line\nbreak",
    ];
    let executed = Command::new(&executable).args(arguments).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(executed.stderr.is_empty());
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        format!(
            "{}\ntrue true\nmutated\n",
            serde_json::to_string(&arguments).unwrap()
        )
    );
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    let prior = std::fs::read(&built).unwrap();
    {
        let source = "import {ChildProcess} from 'node:child_process'; ChildProcess;";
        fixture.write("main.ts", source.as_bytes());
        let (code, _, err) = run(&fixture, &["--out", "native-output"]);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("main.ts:1:"), "{err}");
        assert!(err.contains("ChildProcess"), "{err}");
        assert_eq!(std::fs::read(&built).unwrap(), prior);
    }
}

#[test]
fn native_process_exit_codes_streams_warnings_and_electron_metadata() {
    let fixture = app(r#"process.stdout.write('out:');
process.stdout.write('\u00e9', 'latin1');
process.stdout.write(new Uint8Array([10]));
process.stderr.write('err\n');
const directory = process.execPath.slice(0, process.execPath.lastIndexOf('\\') + 1);
console.log(process.versions.electron, process.type, process.resourcesPath === directory + 'resources');
process.emitWarning('careful', { code: 'W1', detail: 'more' });
process.emitWarning('again');
process.exitCode = 3;
setTimeout(() => {
  console.log('timer');
  if (process.argv[1] === 'exit') {
    process.exit(5);
    console.log('unreachable');
  }
}, 0);
"#);
    fixture.write(
        "node_modules/electron/package.json",
        br#"{"name":"electron","version":"42.1.0"}"#,
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let warnings = |pid: u32| {
        format!(
            "err\n(node:{pid}) [W1] Warning: careful\nmore\n(Use `native-test --trace-warnings ...` to show where the warning was created)\n(node:{pid}) Warning: again\n"
        )
    };
    for (argument, status) in [("normal", 3), ("exit", 5)] {
        let child = Command::new(&executable)
            .arg(argument)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let executed = child.wait_with_output().unwrap();
        assert_eq!(executed.status.code(), Some(status), "{executed:?}");
        assert_eq!(
            executed.stdout,
            b"out:\xe9\n42.1.0 browser true\ntimer\n".to_vec()
        );
        assert_eq!(String::from_utf8(executed.stderr).unwrap(), warnings(pid));
    }
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn child_process_sync_apis_launch_windows_processes_without_node() {
    let fixture = app(
        r#"import {execFileSync,execSync} from 'node:child_process';console.log(execFileSync('where.exe',['cmd.exe'],{cwd:'C:\\Windows\\System32',shell:true}).toLowerCase().includes('cmd.exe'));console.log(JSON.stringify(execSync('echo shell child').trim()));try{execFileSync('where.exe',['nativeboat-command-that-does-not-exist.exe'])}catch(error){console.log(String(error).includes('exit code 1'))}"#,
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "true\n\"shell child\"\ntrue\n"
    );
    assert!(
        executed.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_node_path_from_nodes_own_library_source() {
    // The expected text is Node 22's output for the same program.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-path","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        (
            "main.js",
            r#"'use strict';
const path = require('path');
const { posix, win32 } = path;
const out = [];
const t = (f) => { try { return JSON.stringify(f()); } catch (e) { return `${e.name}:${e.code}:${e.message}`; } };
out.push(path === win32, path.sep, path.delimiter, posix.sep, require('path/posix') === posix, require('node:path/win32') === win32);
out.push(t(() => win32.normalize('C:/a/../中//b\\.\\c\\')), t(() => posix.normalize('/a/../b//c/./')), t(() => win32.join('a', '..', 'b', 'C:\\x')));
out.push(t(() => win32.resolve('C:\\base', 'x', '..', 'y')), t(() => win32.relative('C:\\a\\b\\c', 'C:\\a\\d')), t(() => posix.relative('/a/b/c', '/a/d')));
out.push(t(() => win32.parse('C:\\dir\\file.tar.gz')), t(() => posix.parse('/home/u/.bashrc')), t(() => win32.format({ dir: 'C:\\x', name: 'f', ext: 'txt' })));
out.push(t(() => win32.basename('C:\\a\\b.txt', '.txt')), t(() => win32.extname('a.b.c')), t(() => win32.dirname('\\\\server\\share\\dir\\f')), t(() => win32.isAbsolute('C:foo')));
out.push(t(() => win32.toNamespacedPath('C:\\a')), t(() => win32.toNamespacedPath('\\\\srv\\share\\x')), t(() => posix.toNamespacedPath('/a')));
out.push(t(() => win32.join(1)), t(() => posix.basename('/a', 5)), t(() => win32.format(null)), t(() => win32.format([])));
out.push(t(() => win32.matchesGlob('C:\\a\\b.JS', 'C:/a/*.js')), t(() => posix.matchesGlob('/a/b.js', '/a/**/*.{js,ts}')), t(() => posix.matchesGlob('/a/b.md', '/a/*.js')));
out.push(t(() => path.resolve('rel') === process.cwd() + '\\rel'), t(() => win32.resolve('D:x').startsWith('D:\\')));
console.log(out.join('\n'));
"#,
        ),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-path.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        r#"true
\
;
/
true
true
"C:\\中\\b\\c\\"
"/b/c/"
".\\b\\C:\\x"
"C:\\base\\y"
"..\\..\\d"
"../../d"
{"root":"C:\\","dir":"C:\\dir","base":"file.tar.gz","ext":".gz","name":"file.tar"}
{"root":"/","dir":"/home/u","base":".bashrc","ext":"","name":".bashrc"}
"C:\\x\\f.txt"
"b"
".c"
"\\\\server\\share\\dir"
false
"\\\\?\\C:\\a"
"\\\\?\\UNC\\srv\\share\\x"
"/a"
TypeError:ERR_INVALID_ARG_TYPE:The "path" argument must be of type string. Received type number (1)
TypeError:ERR_INVALID_ARG_TYPE:The "suffix" argument must be of type string. Received type number (5)
TypeError:ERR_INVALID_ARG_TYPE:The "pathObject" argument must be of type object. Received null
TypeError:ERR_INVALID_ARG_TYPE:The "pathObject" argument must be of type object. Received an instance of Array
true
true
false
true
true
"#
    );
    let warning = String::from_utf8(executed.stderr).unwrap();
    assert!(
        warning.ends_with(") ExperimentalWarning: glob is an experimental feature and might change at any time\n(Use `native-path --trace-warnings ...` to show where the warning was created)\n"),
        "{warning}"
    );
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
    // ES modules import the library's CommonJS exports as their default.
    fixture.write("package.json", br#"{"name":"native-path","type":"module","main":"main.js","devDependencies":{"electron":"42"}}"#);
    fixture.write(
        "main.js",
        b"import path from 'node:path'; import { join } from 'path'; import * as ns from 'path'; import win32 from 'path/win32'; console.log(path.join('a', 'b'), win32 === path.win32, path.posix.join('a', 'b'), join === path.join, ns.default === path);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(&executable).output().unwrap();
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "a\\b true a/b true true\n"
    );
}

#[test]
fn builds_node_buffer_from_nodes_own_library_source() {
    // The expected text is Node 22's output for the same program.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-buffer","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        (
            "main.js",
            r##"'use strict';
const { Buffer: Imported, kMaxLength, constants, isUtf8, isAscii, transcode } = require('buffer');
const out = [];
const t = (f) => { try { const v = f(); return typeof v === 'bigint' ? v + 'n' : JSON.stringify(v); } catch (e) { return `${e.name}:${e.code}:${e.message}`; } };
out.push(Imported === Buffer, typeof Buffer, kMaxLength, constants.MAX_STRING_LENGTH, typeof transcode);
const a = Buffer.from('héllo wörld €😀');
out.push(a.length, a.toString(), a.toString('hex'), a.toString('base64'), a.toString('base64url'), Buffer.from(a.toString('latin1'), 'latin1').toString('hex'), Buffer.from(a.toString('ascii'), 'latin1').toString('hex'));
out.push(Buffer.from('68656c6c6f', 'hex').toString(), Buffer.from('aGVsbG8=', 'base64').toString(), Buffer.from('é€', 'latin1').toString('hex'), Buffer.from('abc', 'ucs2').toString('hex'));
out.push(Buffer.byteLength('héllo'), Buffer.byteLength('aGVsbG8=', 'base64'), Buffer.byteLength('abcd', 'hex'), Buffer.byteLength(new Uint8Array(7)));
out.push(Buffer.alloc(4, 'ab').toString(), Buffer.alloc(5, 0x41).toString(), Buffer.alloc(3).toString('hex'), Buffer.allocUnsafe(2).length);
const b = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]);
out.push(b.readUInt16LE(0), b.readUInt16BE(0), b.readInt32LE(1), b.readUInt32BE(4), b.readDoubleLE(0), b.readFloatBE(2), b.readBigUInt64LE(0), b.readBigInt64BE(0), b.readIntLE(0, 6), b.readUIntBE(2, 3));
const w = Buffer.alloc(16);
w.writeUInt32LE(0xdeadbeef, 0); w.writeInt16BE(-2, 4); w.writeDoubleLE(1.5, 6); w.writeBigUInt64BE(2n ** 63n + 5n, 8);
out.push(w.toString('hex'), t(() => w.writeUInt8(300, 0)), t(() => w.readUInt8(16)), t(() => w.writeInt8(1, -1)));
out.push(b.slice(2, 5).toString('hex'), b.subarray(-2).toString('hex'), b.indexOf(3), b.lastIndexOf(8), b.includes(9), Buffer.from('abcabc').indexOf('ca'), Buffer.from('abcabc').lastIndexOf('b'), Buffer.from('abcabc').indexOf(Buffer.from('bc'), 2));
out.push(Buffer.concat([Buffer.from('ab'), Buffer.from('cd')]).toString(), Buffer.concat([Buffer.from('ab'), Buffer.from('cd')], 3).toString(), Buffer.compare(Buffer.from('a'), Buffer.from('b')), Buffer.from('abc').equals(Buffer.from('abc')), Buffer.from('b').compare(Buffer.from('abc')));
const c = Buffer.alloc(6); Buffer.from('xyz').copy(c, 1); out.push(c.toString('hex'), Buffer.from('abcdef').copy(c, 0, 4), c.toString('hex'));
out.push(Buffer.from('abcd').swap16().toString(), Buffer.from('abcd').swap32().toString(), Buffer.from('abcdefgh').swap64().toString(), t(() => Buffer.from('abc').swap16()));
out.push(Buffer.isBuffer(b), Buffer.isBuffer(new Uint8Array(1)), Buffer.isEncoding('UTF-8'), Buffer.isEncoding('nope'), b instanceof Uint8Array, JSON.stringify(Buffer.from('hi')));
out.push(t(() => Buffer.from(5)), t(() => Buffer.alloc(-1)), t(() => Buffer.from('x', 'nope')), t(() => b.toString('nope')), t(() => Buffer.concat('x')), t(() => b.fill('zz', 'hex')));
out.push(isUtf8(Buffer.from([0xc3, 0xa9])), isUtf8(Buffer.from([0xff])), isAscii(Buffer.from('abc')), isAscii(Buffer.from('é')));
out.push(atob('aGVsbG8='), btoa('hello'), t(() => atob('*')), t(() => btoa('€')));
out.push(Buffer.from(new Uint16Array([1, 256]).buffer).toString('hex'), Buffer.from(Buffer.from('abc')).toString(), Buffer.from({ length: 2, 0: 65, 1: 66 }).toString(), b.toJSON().data.length);
const filled = Buffer.alloc(7); filled.fill('ab', 1, 6); out.push(filled.toString('hex'), Buffer.alloc(4).fill(257).toString('hex'), Buffer.alloc(3).fill('é').toString('hex'));
const written = Buffer.alloc(6); out.push(written.write('héllo'), written.toString(), written.write('ab', 4, 'latin1'), written.write('6162', 0, 1, 'hex'), written.toString('hex'));
out.push(Buffer.from('test').toString('utf8', 1, 3), Buffer.from('€').toString('utf8', 0, 2), String(Buffer.from('xy')), Buffer.from('a').toLocaleString());
out.push([...Buffer.from('ab')].join(), [...Buffer.from('ab').entries()].join(';'), Buffer.from('hello').readUIntLE(0, 5));
const tc = (f) => { try { return f().toString('hex'); } catch (e) { return e.name + ':' + e.code + ':' + e.errno + ':' + e.message; } };
const src = Buffer.from('aé€😀ÿ');
for (const [f, to, input] of [['utf8', 'latin1', src], ['utf8', 'ascii', src], ['utf8', 'ucs2', src], ['ucs2', 'utf8', Buffer.from('aé€😀', 'ucs2')], ['latin1', 'utf8', Buffer.from([0x61, 0xe9, 0xff])], ['ascii', 'utf8', Buffer.from([0x61, 0xe9])], ['utf8', 'utf8', src], ['latin1', 'ucs2', Buffer.from([0x61, 0xe9, 0xff])], ['utf8', 'binary', src], ['utf8', 'hex', src], ['utf8', 'ucs2', Buffer.from([0xff, 0xfe, 0x41])], ['ucs2', 'utf8', Buffer.from([0x41, 0x00, 0x00, 0xd8])], ['ucs2', 'latin1', Buffer.from('aé€', 'ucs2')]]) out.push(tc(() => transcode(input, f, to)));
out.push(tc(() => transcode('x', 'utf8', 'ucs2')), tc(() => transcode(Buffer.alloc(0), 'utf8', 'ucs2')), Buffer.isBuffer(transcode(src, 'utf8', 'ucs2')), transcode(new Uint8Array([97]), 'latin1', 'utf8') instanceof Buffer);
console.log(out.join('\n'));
"##,
        ),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-buffer.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        r##"true
function
9007199254740991
536870888
function
21
héllo wörld €😀
68c3a96c6c6f2077c3b6726c6420e282acf09f9880
aMOpbGxvIHfDtnJsZCDigqzwn5iA
aMOpbGxvIHfDtnJsZCDigqzwn5iA
68c3a96c6c6f2077c3b6726c6420e282acf09f9880
6843296c6c6f20774336726c642062022c701f1800
hello
hello
e9ac
610062006300
6
5
2
7
abab
AAAAA
000000
2
513
258
84148994
84281096
5.447603722011605e-270
3.879708020057588e-37
578437695752307201
72623859790382856
6618611909121
197637
efbeaddefffe00008000000000000005
RangeError:ERR_OUT_OF_RANGE:The value of "value" is out of range. It must be >= 0 and <= 255. Received 300
RangeError:ERR_OUT_OF_RANGE:The value of "offset" is out of range. It must be >= 0 and <= 15. Received 16
RangeError:ERR_OUT_OF_RANGE:The value of "offset" is out of range. It must be >= 0 and <= 15. Received -1
030405
0708
2
7
false
2
4
4
abcd
abc
-1
true
1
0078797a0000
2
6566797a0000
badc
dcba
hgfedcba
RangeError:ERR_INVALID_BUFFER_SIZE:Buffer size must be a multiple of 16-bits
true
false
true
false
true
{"type":"Buffer","data":[104,105]}
TypeError:ERR_INVALID_ARG_TYPE:The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object. Received type number (5)
RangeError:ERR_OUT_OF_RANGE:The value of "size" is out of range. It must be >= 0 && <= 9007199254740991. Received -1
TypeError:ERR_UNKNOWN_ENCODING:Unknown encoding: nope
TypeError:ERR_UNKNOWN_ENCODING:Unknown encoding: nope
TypeError:ERR_INVALID_ARG_TYPE:The "list" argument must be an instance of Array. Received type string ('x')
TypeError:ERR_INVALID_ARG_VALUE:The argument 'value' is invalid. Received 'zz'
true
false
true
false
hello
aGVsbG8=
InvalidCharacterError:5:Invalid character
InvalidCharacterError:5:Invalid character
01000001
abc
AB
8
00616261626100
01010101
c3a9c3
6
héllo
2
1
61c3a96c6162
es
�
xy
a
97,98
0,97;1,98
478560413032
61e93f3fff
613f3f3f3f
6100e900ac203dd800deff00
61c3a9e282acf09f9880
61c3a9c3bf
61efbfbd
61c3a9e282acf09f9880c3bf
6100e900ff00
61e93f3fff
Error:U_ILLEGAL_ARGUMENT_ERROR:1:Unable to transcode Buffer [U_ILLEGAL_ARGUMENT_ERROR]
Error:U_INVALID_CHAR_FOUND:10:Unable to transcode Buffer [U_INVALID_CHAR_FOUND]
Error:U_INVALID_CHAR_FOUND:10:Unable to transcode Buffer [U_INVALID_CHAR_FOUND]
61e93f
TypeError:ERR_INVALID_ARG_TYPE:undefined:The "source" argument must be an instance of Buffer or Uint8Array. Received type string ('x')

true
true
"##
    );
    // ES modules read Buffer, atob and btoa as globals.
    fixture.write(
        "package.json",
        br#"{"name":"native-buffer","type":"module","main":"main.js","devDependencies":{"electron":"42"}}"#,
    );
    fixture.write(
        "main.js",
        b"console.log(Buffer.from('hi').toString('hex'), typeof atob, btoa('a'), globalThis.Buffer === Buffer, Object.getOwnPropertyDescriptor(globalThis, 'atob').enumerable);",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(&executable).output().unwrap();
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "6869 function YQ== true true\n"
    );
}

#[test]
fn builds_node_events_from_nodes_own_library_source() {
    // The expected text is Node 22's output for the same program.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-events","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        (
            "main.js",
            r##"'use strict';
const EventEmitter = require('events');
const { once, errorMonitor, getEventListeners, getMaxListeners, setMaxListeners, listenerCount } = require('events');
const out = [];
const t = (f) => { try { return String(f()); } catch (e) { return `${e.name}:${e.code}:${e.message}`; } };

const e = new EventEmitter();
const log = [];
const a = (x) => log.push(`a${x}`);
const b = (x) => log.push(`b${x}`);
e.on('x', a);
e.prependListener('x', b);
e.once('x', (x) => log.push(`once${x}`));
out.push(e.emit('x', 1), e.emit('x', 2), e.emit('nothing'), log.join());
out.push(e.listenerCount('x'), EventEmitter.listenerCount(e, 'x'), e.eventNames().join(), e.rawListeners('x').length, e.listeners('x').length);
e.off('x', a); e.removeListener('x', b);
out.push(e.listenerCount('x'), e.eventNames().length);
const symbol = Symbol('s');
e.on(symbol, () => log.push('symbol'));
e.emit(symbol);
out.push(log.at(-1), String(e.eventNames()[0]));
out.push(t(() => e.emit('error', new Error('boom'))), t(() => e.emit('error', 'plain')), t(() => e.emit('error')));
const monitored = [];
e.on(errorMonitor, (err) => monitored.push(err.message));
e.on('error', (err) => monitored.push(`handled ${err.message}`));
e.emit('error', new Error('seen'));
out.push(monitored.join());
out.push(EventEmitter.defaultMaxListeners, e.getMaxListeners(), t(() => e.setMaxListeners(-1)), t(() => e.on('x', 5)));
class Custom extends EventEmitter {
  constructor() { super(); this.name = 'custom'; }
}
const c = new Custom();
c.on('ready', function () { out.push(`ready ${this.name} ${this instanceof EventEmitter}`); });
c.emit('ready');
out.push(getEventListeners(c, 'ready').length, getMaxListeners(c), listenerCount(c, 'ready'));
setMaxListeners(1, c);
c.on('ready', () => {});
c.removeAllListeners('ready');
out.push(c.listenerCount('ready'));
const nl = [];
e.on('newListener', (name) => nl.push(String(name)));
e.on('removeListener', (name) => nl.push(`-${String(name)}`));
const h = () => {};
e.on('z', h); e.off('z', h);
out.push(nl.join());
(async () => {
  const pending = once(e, 'later');
  setTimeout(() => e.emit('later', 1, 2), 0);
  out.push((await pending).join());
  const failing = once(e, 'never');
  setTimeout(() => e.emit('error', new Error('rejected')), 0);
  try { await failing; } catch (err) { out.push(`once rejected ${err.message}`); }
  console.log(out.join('\n'));
})();
"##,
        ),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-events.exe");
    let child = Command::new(&executable)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let executed = child.wait_with_output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        r##"true
true
false
b1,a1,once1,b2,a2
2
2
x
2
2
0
0
symbol
Symbol(s)
Error:undefined:boom
Error:ERR_UNHANDLED_ERROR:Unhandled error. ('plain')
Error:ERR_UNHANDLED_ERROR:Unhandled error. (undefined)
seen,handled seen
10
10
RangeError:ERR_OUT_OF_RANGE:The value of "setMaxListeners" is out of range. It must be >= 0. Received -1
TypeError:ERR_INVALID_ARG_TYPE:The "listener" argument must be of type function. Received type number (5)
ready custom true
1
10
1
0
removeListener,z,-z
1,2
once rejected rejected
"##
    );
    assert_eq!(
        String::from_utf8(executed.stderr).unwrap(),
        format!(
            "(node:{pid}) MaxListenersExceededWarning: Possible EventEmitter memory leak detected. 2 ready listeners added to [Custom]. MaxListeners is 1. Use emitter.setMaxListeners() to increase limit\n(Use `native-events --trace-warnings ...` to show where the warning was created)\n"
        )
    );
}

#[test]
fn builds_process_events_like_node() {
    // The expected text is Node 22's output for the same program.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-process","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        (
            "main.js",
            r##"'use strict';
const EventEmitter = require('events');
const proc = require('process');
const log = (...a) => console.log(...a);
log(proc === process, process instanceof EventEmitter, Object.prototype.toString.call(process), process._exiting);
log(process.on('custom', (x) => log('custom', x)) === process, process.listenerCount('custom'), process.emit('custom', 1), process.eventNames().join());
log(process.listeners('warning').map((f) => f.name).join(), process.listeners('newListener').map((f) => f.name).join());
process.removeAllListeners('warning');
process.on('warning', (w) => log('warning', w.name, w.message));
process.emitWarning('careful');
process.on('uncaughtExceptionMonitor', (e, origin) => log('monitor', e.message, origin));
let count = 0;
process.on('uncaughtException', (e, origin) => { log('caught', e.message, origin); count++; });
process.on('unhandledRejection', (reason, promise) => {
  log('unhandled', String(reason), promise instanceof Promise);
  if (reason === 'first') later.catch(() => log('handled before reported'));
});
Promise.reject('first');
const later = Promise.reject('second');
Promise.reject('third');
process.once('beforeExit', (code) => {
  log('beforeExit', code, count);
  setTimeout(() => { throw new Error('in beforeExit timer'); }, 0);
});
process.on('beforeExit', (code) => log('beforeExit again', code));
process.on('exit', (code) => { log('exit', code, process._exiting, process.exitCode); process.exitCode = 4; setTimeout(() => log('never'), 0); });
setTimeout(() => {
  setImmediate(() => { throw new Error('immediate 1'); });
  setImmediate(() => log('immediate 2'));
  throw new Error('timer');
}, 0);
queueMicrotask(() => { throw new Error('microtask'); });
process.nextTick(() => { throw new Error('tick'); });
process.nextTick(() => log('tick 2'));
(async () => { throw new Error('async'); })();
throw new Error('main');
"##,
        ),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-process.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert_eq!(executed.status.code(), Some(4), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        r##"true true [object process] false
custom 1
true 1 true newListener,removeListener,warning,custom
onWarning ,startListeningIfSignal
monitor main uncaughtException
caught main uncaughtException
warning Warning careful
monitor tick uncaughtException
caught tick uncaughtException
monitor timer uncaughtException
caught timer uncaughtException
tick 2
monitor microtask uncaughtException
caught microtask uncaughtException
unhandled first true
unhandled second true
unhandled third true
unhandled Error: async true
handled before reported
monitor immediate 1 uncaughtException
caught immediate 1 uncaughtException
immediate 2
beforeExit 0 5
beforeExit again 0
monitor in beforeExit timer uncaughtException
caught in beforeExit timer uncaughtException
beforeExit again 0
exit 0 true undefined
"##
    );
    assert!(executed.stderr.is_empty());
    // An unhandled exception emits `exit` with 1 before it is reported.
    fixture.write(
        "package.json",
        br#"{"name":"native-process","main":"main.mjs","devDependencies":{"electron":"42"}}"#,
    );
    fixture.write(
        "main.mjs",
        b"import process, { platform } from 'node:process';\n\
          process.on('exit', (code) => console.log('exit', code, platform));\n\
          throw new Error('plain');\n",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(&executable).output().unwrap();
    assert_eq!(executed.status.code(), Some(1), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "exit 1 win32\n"
    );
    assert_eq!(
        String::from_utf8(executed.stderr).unwrap(),
        "Uncaught Error: plain\n"
    );
}

#[test]
fn builds_node_os_from_nodes_own_library_source() {
    // The expected text is Node 22's output for the same program; it holds
    // on any Windows machine.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-os","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        (
            "main.js",
            r##"'use strict';
const os = require('os');
const { EOL, constants, devNull } = require('node:os');
const out = [];
const show = (value) => JSON.stringify(value);
const t = (f) => { try { return show(f()); } catch (e) { return `${e.name}:${e.code}:${e.message}`; } };
const env = process.env;
out.push(Object.keys(os).join());
out.push(show([os.arch(), os.platform(), os.type(), os.machine(), os.endianness(), EOL, devNull, os.loadavg()]));
out.push(show([`${os.arch}`, `${os.platform}`, `${os.type}`, `${os.machine}`, `${os.endianness}`]));
out.push(/^\d+\.\d+\.\d+$/.test(os.release()), os.version().startsWith('Windows'), `${os.release}` === os.release());
out.push(os.homedir() === env.USERPROFILE, `${os.homedir}` === os.homedir());
const temp = env.TEMP || env.TMP || `${env.SystemRoot || env.windir}\\temp`;
out.push(os.tmpdir() === (temp.length > 1 && temp.endsWith('\\') && !temp.endsWith(':\\') ? temp.slice(0, -1) : temp));
out.push(typeof os.hostname() === 'string' && os.hostname().length > 0, `${os.hostname}` === os.hostname());
const user = os.userInfo();
out.push(show([user.uid, user.gid, user.shell, Object.keys(user)]), Object.getPrototypeOf(user) === null);
out.push(user.username.toLowerCase() === env.USERNAME.toLowerCase(), user.homedir.length > 0);
const buffered = os.userInfo({ encoding: 'buffer' });
out.push(Buffer.isBuffer(buffered.username), buffered.username.toString() === user.username, buffered.shell);
out.push(os.userInfo({ encoding: 'hex' }).username === Buffer.from(user.username).toString('hex'));
out.push(show(constants), Object.getPrototypeOf(constants.signals), Object.isFrozen(constants));
const cpus = os.cpus();
out.push(cpus.length > 0, cpus.length >= os.availableParallelism());
out.push(cpus.every((cpu) => typeof cpu.model === 'string' && cpu.speed >= 0 && Object.keys(cpu.times).join() === 'user,nice,sys,idle,irq' && cpu.times.nice === 0));
out.push(os.totalmem() > os.freemem(), os.freemem() > 0, Number.isInteger(os.totalmem()), os.uptime() > 0);
const entries = Object.values(os.networkInterfaces()).flat();
out.push(entries.some((entry) => entry.internal && entry.address === '127.0.0.1'));
out.push(entries.every((entry) => {
  const keys = Object.keys(entry).join();
  return (entry.family === 'IPv6' ? keys === 'address,netmask,family,mac,internal,cidr,scopeid' : keys === 'address,netmask,family,mac,internal,cidr') &&
    entry.cidr.startsWith(`${entry.address}/`) && /^([0-9a-f]{2}:){5}[0-9a-f]{2}$/.test(entry.mac);
}));
out.push([-20, -14, -7, 0, 10, 19].includes(os.getPriority()), os.getPriority(0) === os.getPriority(), t(() => os.getPriority(-1.5)), t(() => os.getPriority('x')), t(() => os.setPriority(0, 100)), t(() => os.setPriority(0, 0)));
try { os.getPriority(2147483647); } catch (e) {
  out.push(e.name, e.code, e.message, show(e.info), e.errno, e.syscall, Object.keys(e).join(), String(e));
  e.errno = 5;
  out.push(e.info.errno);
}
console.log(out.join('\n'));
"##,
        ),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-os.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        r##"arch,availableParallelism,cpus,endianness,freemem,getPriority,homedir,hostname,loadavg,networkInterfaces,platform,release,setPriority,tmpdir,totalmem,type,userInfo,uptime,version,machine,constants,EOL,devNull
["x64","win32","Windows_NT","x86_64","LE","\r\n","\\\\.\\nul",[0,0,0]]
["x64","win32","Windows_NT","x86_64","LE"]
true
true
true
true
true
true
true
true
[-1,-1,null,["uid","gid","username","homedir","shell"]]
true
true
true
true
true

true
{"UV_UDP_REUSEADDR":4,"dlopen":{},"errno":{"E2BIG":7,"EACCES":13,"EADDRINUSE":100,"EADDRNOTAVAIL":101,"EAFNOSUPPORT":102,"EAGAIN":11,"EALREADY":103,"EBADF":9,"EBADMSG":104,"EBUSY":16,"ECANCELED":105,"ECHILD":10,"ECONNABORTED":106,"ECONNREFUSED":107,"ECONNRESET":108,"EDEADLK":36,"EDESTADDRREQ":109,"EDOM":33,"EEXIST":17,"EFAULT":14,"EFBIG":27,"EHOSTUNREACH":110,"EIDRM":111,"EILSEQ":42,"EINPROGRESS":112,"EINTR":4,"EINVAL":22,"EIO":5,"EISCONN":113,"EISDIR":21,"ELOOP":114,"EMFILE":24,"EMLINK":31,"EMSGSIZE":115,"ENAMETOOLONG":38,"ENETDOWN":116,"ENETRESET":117,"ENETUNREACH":118,"ENFILE":23,"ENOBUFS":119,"ENODATA":120,"ENODEV":19,"ENOENT":2,"ENOEXEC":8,"ENOLCK":39,"ENOLINK":121,"ENOMEM":12,"ENOMSG":122,"ENOPROTOOPT":123,"ENOSPC":28,"ENOSR":124,"ENOSTR":125,"ENOSYS":40,"ENOTCONN":126,"ENOTDIR":20,"ENOTEMPTY":41,"ENOTSOCK":128,"ENOTSUP":129,"ENOTTY":25,"ENXIO":6,"EOPNOTSUPP":130,"EOVERFLOW":132,"EPERM":1,"EPIPE":32,"EPROTO":134,"EPROTONOSUPPORT":135,"EPROTOTYPE":136,"ERANGE":34,"EROFS":30,"ESPIPE":29,"ESRCH":3,"ETIME":137,"ETIMEDOUT":138,"ETXTBSY":139,"EWOULDBLOCK":140,"EXDEV":18,"WSAEINTR":10004,"WSAEBADF":10009,"WSAEACCES":10013,"WSAEFAULT":10014,"WSAEINVAL":10022,"WSAEMFILE":10024,"WSAEWOULDBLOCK":10035,"WSAEINPROGRESS":10036,"WSAEALREADY":10037,"WSAENOTSOCK":10038,"WSAEDESTADDRREQ":10039,"WSAEMSGSIZE":10040,"WSAEPROTOTYPE":10041,"WSAENOPROTOOPT":10042,"WSAEPROTONOSUPPORT":10043,"WSAESOCKTNOSUPPORT":10044,"WSAEOPNOTSUPP":10045,"WSAEPFNOSUPPORT":10046,"WSAEAFNOSUPPORT":10047,"WSAEADDRINUSE":10048,"WSAEADDRNOTAVAIL":10049,"WSAENETDOWN":10050,"WSAENETUNREACH":10051,"WSAENETRESET":10052,"WSAECONNABORTED":10053,"WSAECONNRESET":10054,"WSAENOBUFS":10055,"WSAEISCONN":10056,"WSAENOTCONN":10057,"WSAESHUTDOWN":10058,"WSAETOOMANYREFS":10059,"WSAETIMEDOUT":10060,"WSAECONNREFUSED":10061,"WSAELOOP":10062,"WSAENAMETOOLONG":10063,"WSAEHOSTDOWN":10064,"WSAEHOSTUNREACH":10065,"WSAENOTEMPTY":10066,"WSAEPROCLIM":10067,"WSAEUSERS":10068,"WSAEDQUOT":10069,"WSAESTALE":10070,"WSAEREMOTE":10071,"WSASYSNOTREADY":10091,"WSAVERNOTSUPPORTED":10092,"WSANOTINITIALISED":10093,"WSAEDISCON":10101,"WSAENOMORE":10102,"WSAECANCELLED":10103,"WSAEINVALIDPROCTABLE":10104,"WSAEINVALIDPROVIDER":10105,"WSAEPROVIDERFAILEDINIT":10106,"WSASYSCALLFAILURE":10107,"WSASERVICE_NOT_FOUND":10108,"WSATYPE_NOT_FOUND":10109,"WSA_E_NO_MORE":10110,"WSA_E_CANCELLED":10111,"WSAEREFUSED":10112},"signals":{"SIGHUP":1,"SIGINT":2,"SIGQUIT":3,"SIGILL":4,"SIGABRT":22,"SIGFPE":8,"SIGKILL":9,"SIGSEGV":11,"SIGTERM":15,"SIGBREAK":21,"SIGWINCH":28},"priority":{"PRIORITY_LOW":19,"PRIORITY_BELOW_NORMAL":10,"PRIORITY_NORMAL":0,"PRIORITY_ABOVE_NORMAL":-7,"PRIORITY_HIGH":-14,"PRIORITY_HIGHEST":-20}}

false
true
true
true
true
true
true
true
true
true
true
true
RangeError:ERR_OUT_OF_RANGE:The value of "pid" is out of range. It must be an integer. Received -1.5
TypeError:ERR_INVALID_ARG_TYPE:The "pid" argument must be of type number. Received type string ('x')
RangeError:ERR_OUT_OF_RANGE:The value of "priority" is out of range. It must be >= -20 && <= 19. Received 100

SystemError
ERR_SYSTEM_ERROR
A system error occurred: uv_os_getpriority returned ESRCH (no such process)
{"errno":-4040,"code":"ESRCH","message":"no such process","syscall":"uv_os_getpriority"}
-4040
uv_os_getpriority
code,info,errno,syscall
SystemError [ERR_SYSTEM_ERROR]: A system error occurred: uv_os_getpriority returned ESRCH (no such process)
5
"##
    );
    assert!(executed.stderr.is_empty());
    // Core runtime imports kernel32, the OS crypto providers and user32 for
    // the linked native-keymap adapter.
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_ajv_schema_validation_without_code_generation() {
    // The expected text is Node 22's output for the same program with the
    // replacement sources, which match Ajv 8.20's results. The package files
    // throw: Ajv's entry files and ajv-formats' plugin are never compiled.
    let generator = b"throw new Error('Ajv code generation was compiled');";
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-ajv","main":"main.mjs","devDependencies":{"electron":"42"}}"#,
        ),
        ("main.mjs", include_str!("tests/json_schema.mjs")),
        (
            "node_modules/ajv/package.json",
            r#"{"name":"ajv","version":"8.20.0","main":"dist/ajv.js"}"#,
        ),
        (
            "node_modules/ajv-formats/package.json",
            r#"{"name":"ajv-formats","version":"3.0.1","main":"dist/index.js"}"#,
        ),
        (
            "node_modules/ajv-formats/dist/formats.js",
            include_str!("tests/json_schema_formats.cjs"),
        ),
    ]);
    for file in [
        "node_modules/ajv/dist/ajv.js",
        "node_modules/ajv/dist/2020.js",
        "node_modules/ajv-formats/dist/index.js",
    ] {
        fixture.write(file, generator);
    }
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-ajv.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/json_schema.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty());
}

#[test]
fn builds_fflates_asynchronous_api_without_workers() {
    // The expected text is Node 22's output for the same program with the
    // replacement source. fflate's own entry is never compiled: it throws.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-fflate","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        ("main.js", include_str!("tests/fflate_async.js")),
        (
            "node_modules/fflate/package.json",
            r#"{"name":"fflate","version":"0.8.2","main":"./lib/index.cjs","exports":{".":{"node":{"require":"./lib/node.cjs"},"require":"./lib/index.cjs"}}}"#,
        ),
        (
            "node_modules/fflate/lib/index.cjs",
            include_str!("tests/fflate_sync.cjs"),
        ),
        (
            "node_modules/fflate/lib/node.cjs",
            "throw new Error('fflate workers were compiled');",
        ),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-fflate.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/fflate_async.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty());
}

#[test]
fn builds_weak_collections_like_node() {
    // The expected text is Node 22's output for the same program.
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-weak","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        ("main.js", include_str!("tests/weak_collections.js")),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-weak.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/weak_collections.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_bigint_typed_arrays_and_exact_data_view_operations() {
    let fixture = app(include_str!("tests/bigint_typed_arrays.js"));
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        include_str!("tests/bigint_typed_arrays.expected.txt").replace("\r\n", "\n")
    );
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn builds_unchanged_filesystem_metadata_helpers_and_constructor_wrappers() {
    build_node_helpers(
        include_str!("tests/fs_utils.cjs"),
        include_str!("tests/fs_utils.expected.txt"),
    );
}

#[test]
fn builds_filesystem_uv_errors_with_native_fields_and_observable_order() {
    build_node_helpers(
        include_str!("tests/fs_errors.cjs"),
        include_str!("tests/fs_errors.expected.txt"),
    );
}

#[test]
fn builds_packaged_native_io_workers_without_node_or_crt() {
    // Exercise the native platform facility before exposing public Node fs.
    // This Rust probe verifies packaged sources and worker DLL dependencies;
    // it is not a JavaScript filesystem compilation or installation receipt.
    let fixture = Fixture::new(&[("stat-中🚀.txt", "nativeboat")]);
    let tool = Toolchain {
        rustc: PathBuf::from("rustc"),
        sysroot: None,
    };
    let cache = cache_directory();
    let key = toolchain_key(&tool, &cache).unwrap();
    let (matcher, _matcher_copy) = matcher(&tool, &cache, key).unwrap();
    let (url, _url_copy) = url_parser(&tool, &cache, key).unwrap();
    let (encoding, _encoding_copy) = encoding_detector(&tool, &cache, key, &matcher).unwrap();
    let (runtime, _runtime_copy) =
        runtime(&tool, &cache, key, (&matcher, &url, &encoding), None).unwrap();
    let source = include_str!("tests/io_workers.rs").replace(
        "__STAT_PATH__",
        &format!("{:?}", fixture.path("stat-中🚀.txt").to_str().unwrap()),
    );
    fixture.write("app.rs", source.as_bytes());
    let mut arguments: Vec<OsString> = [
        "app.rs",
        "--edition=2024",
        "--crate-name=nativeboat_app",
        "-Cpanic=abort",
        "-Copt-level=2",
        "-Dwarnings",
        "-Clinker=rust-lld",
        "-Clinker-flavor=lld-link",
        "-Clink-arg=/NODEFAULTLIB",
        "-Clink-arg=/ENTRY:nativeboat_start",
        "-Clink-arg=/SUBSYSTEM:CONSOLE",
        "-Clink-arg=/STACK:268435456",
        "-o",
        "app.exe",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    for name in ["nativeboat_native", "nativeboat_start"] {
        let mut argument = OsString::from(format!("--extern={name}="));
        argument.push(runtime.join(format!("lib{name}.rlib")));
        arguments.push(argument);
    }
    for (prefix, path) in [
        ("-L", &runtime),
        ("-Ldependency=", &matcher),
        ("-Ldependency=", &url),
        ("-Ldependency=", &encoding),
    ] {
        let mut argument = OsString::from(prefix);
        argument.push(path);
        arguments.push(argument);
    }
    toolchain(&tool, &arguments, fixture.root()).unwrap();
    let executable = fixture.path("app.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "worker true completion true size 10\n"
    );
    assert!(executed.stderr.is_empty());
    assert!(imported_dlls(&executable).iter().all(|name| {
        [
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "kernel32.dll",
            "user32.dll",
            "winhttp.dll",
            "ws2_32.dll",
        ]
        .contains(&name.as_str())
    }));
}

#[test]
fn staging_directories_of_killed_builds_are_removed() {
    let fixture = Fixture::new(&[]);
    let out = fixture.path("out");
    // An exited process whose handle is still held keeps its id unused.
    let mut exited = std::process::Command::new("cmd")
        .args(["/c", "exit"])
        .spawn()
        .unwrap();
    exited.wait().unwrap();
    let dead = exited.id();
    let own = std::process::id();
    let directories = [
        (format!(".nativeboat-build-{dead}-0"), false),
        (format!(".nativeboat-build-{own}-1"), true),
        (".nativeboat-build-x-1".to_string(), true),
        (format!(".nativeboat-build-{dead}-z"), true),
        (format!("kept-{dead}-0"), true),
        (".short".to_string(), true),
    ];
    for (name, _) in &directories {
        std::fs::create_dir_all(out.join(name)).unwrap();
    }
    std::fs::write(out.join(format!(".nativeboat-build-{dead}-2")), "").unwrap();
    remove_abandoned_staging(&out, ".nativeboat-build-");
    for (name, kept) in &directories {
        assert_eq!(out.join(name).exists(), *kept, "{name}");
    }
    assert!(out.join(format!(".nativeboat-build-{dead}-2")).is_file());
    // Processes that cannot be opened are taken as running; no process has
    // the id 0.
    assert!(process_running(own));
    assert!(process_running(4));
    assert!(!process_running(0));
    assert!(!process_running(dead));
    drop(exited);
}

#[test]
fn a_link_map_is_written_only_on_request() {
    assert!(link_map(None).is_empty());
    assert_eq!(
        link_map(Some("C:/maps/app.map".into())),
        [OsString::from("-Clink-arg=/MAP:C:/maps/app.map")]
    );
}

#[test]
fn module_maps_name_the_source_of_each_module_function() {
    let fixture = Fixture::new(&[]);
    let path = fixture.path("modules.txt");
    let modules = [
        vec![
            "pub fn P3_0_f0() {}".to_string(),
            "\n/// `src/a.js`\nfn m3(realm: &mut Realm) {}".to_string(),
        ],
        vec!["fn helper() {}".to_string()],
    ];
    let units = ModuleUnits::of(&modules);
    let chunks = [Some(1), Some(2), None];
    write_module_map(None, "renderer_0", &units, &chunks).unwrap();
    assert!(!path.exists());
    write_module_map(Some(path.as_os_str()), "renderer_0", &units, &chunks).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "renderer_0 m3 src/a.js 62 chunk 2 parts 2\nrenderer_0 m? ? 14 chunk -\n"
    );
    assert!(
        write_module_map(
            Some(fixture.path("missing/x.txt").as_os_str()),
            "g",
            &units,
            &chunks
        )
        .is_err()
    );
}

#[test]
fn stylesheets_carry_the_local_files_they_reference() {
    let fixture = Fixture::new(&[]);
    fixture.write("styles/fonts/face.woff", b"wOFF-bytes");
    fixture.write("styles/icon.svg", b"<svg/>");
    fixture.write("styles/notes.txt", b"text");
    let css = "@font-face { src: url('./fonts/face.woff?v=1#x') format('woff'); }\n\
               a { background: url(icon.svg) } b { background: url(\"missing.png\") }\n\
               c { background: url(data:image/png;base64,AA==) } d { background: url(https://example.com/x.png) }\n\
               e { background: url(notes.txt) } f { clip-path: url(#shape) } g { background: url(";
    let inlined = inline_css_assets(&fixture.path("styles/main.css"), css);
    assert!(
        inlined.contains("url(\"data:font/woff;base64,d09GRi1ieXRlcw==\") format('woff')"),
        "{inlined}"
    );
    assert!(
        inlined.contains("url(\"data:image/svg+xml;base64,PHN2Zy8+\")"),
        "{inlined}"
    );
    for unchanged in [
        "url(\"missing.png\")",
        "url(data:image/png;base64,AA==)",
        "url(https://example.com/x.png)",
        "url(notes.txt)",
        "url(#shape)",
    ] {
        assert!(inlined.contains(unchanged), "{unchanged} in {inlined}");
    }
    assert!(inlined.ends_with("g { background: url("));
}

#[test]
fn compiled_programs_match_node_on_the_conformance_script() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"conformance","main":"main.js","devDependencies":{"electron":"44"}}"#,
        ),
        ("main.js", include_str!("../../tests/conformance/main.js")),
    ]);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/conformance.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    let actual = String::from_utf8_lossy(&executed.stdout).replace("\r\n", "\n");
    let expected = include_str!("../../tests/conformance/expected.txt").replace("\r\n", "\n");
    for (number, (actual, expected)) in actual.lines().zip(expected.lines()).enumerate() {
        assert_eq!(actual, expected, "line {}", number + 1);
    }
    assert_eq!(actual.lines().count(), expected.lines().count());
}

#[test]
fn cache_entries_in_use_by_a_build_survive_newer_builds() {
    let fixture = Fixture::new(&[]);
    let cache = fixture.path("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let make = |name: &str| {
        let directory = cache.join(name);
        std::fs::create_dir(&directory).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        directory
    };
    // A long build's runtime, used just now, then newer builds of others.
    let busy = make("runtime-busy");
    mark_used(&busy);
    let stale = make("runtime-stale");
    mark_used(&stale);
    let marker = std::fs::File::options()
        .write(true)
        .open(stale.join(LAST_USE))
        .unwrap();
    marker
        .set_modified(std::time::SystemTime::now() - CACHE_IN_USE * 2)
        .unwrap();
    drop(marker);
    for index in 0..CACHE_KEEP {
        make(&format!("runtime-new{index}"));
    }
    prune_unlocked(&cache, "runtime-new0");
    assert!(busy.exists(), "a library a build uses stays");
    assert!(!stale.exists(), "one unused for long goes");
    assert!((0..CACHE_KEEP).all(|index| cache.join(format!("runtime-new{index}")).exists()));
    assert!(last_use(&busy).is_some() && last_use(&cache.join("runtime-new0")).is_none());
    // Generated crates keep only their newest build, used or not.
    let old = make("generated_a-1");
    mark_used(&old);
    std::thread::sleep(std::time::Duration::from_millis(50));
    make("generated_a-2");
    prune_unlocked(&cache, "generated_a-2");
    assert!(!old.exists() && cache.join("generated_a-2").exists());
}

#[test]
fn readline_http2_net_and_tls_modules_load_with_their_node_surface() {
    let fixture = app(r#"import readline from 'node:readline';
import { Readable } from 'node:stream';
import net from 'node:net';
import tls from 'node:tls';
import http2 from 'node:http2';
const packed = http2.getPackedSettings({ maxConcurrentStreams: 100, enablePush: false });
console.log(net.isIP('::1'), net.isIPv4('1.2.3.4'), net.isIP('host'), http2.constants.HTTP2_HEADER_STATUS,
  packed.toString('hex'), JSON.stringify(http2.getUnpackedSettings(packed)), tls.DEFAULT_MIN_VERSION);
const lines = readline.createInterface({ input: Readable.from(['first\nsec', 'ond\n']) });
lines.on('line', (line) => console.log('line', line));
lines.on('close', () => {
  // A port nothing listens on any more refuses the connection.
  const probe = net.createServer().listen(0, '127.0.0.1', () => {
    const port = probe.address().port;
    probe.close(() => {
      const socket = net.connect(port, '127.0.0.1');
      socket.on('error', (error) => console.log('connect', error.code, socket.destroyed));
    });
  });
});
"#);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "6 true 0 :status 000200000000000300000064 {\"enablePush\":false,\"maxConcurrentStreams\":100} TLSv1.2\nline first\nline second\nconnect ECONNREFUSED true\n",
        "{executed:?}"
    );
}

#[test]
fn async_local_storage_follows_awaits_timers_ticks_and_io() {
    let fixture = app(
        r#"import { AsyncLocalStorage, AsyncResource } from 'node:async_hooks';
import fs from 'node:fs';
const storage = new AsyncLocalStorage();
const seen = [];
const resource = storage.run('bound', () => new AsyncResource('test'));
async function main() {
  await storage.run('outer', async () => {
    seen.push(storage.getStore());
    await Promise.resolve();
    seen.push(storage.getStore());
    await new Promise((resolve) => setTimeout(() => { seen.push(storage.getStore()); resolve(); }, 1));
    process.nextTick(() => seen.push('tick ' + storage.getStore()));
    await new Promise((resolve) => setImmediate(resolve));
    await new Promise((resolve) => fs.readFile(process.execPath, () => { seen.push('io ' + storage.getStore()); resolve(); }));
    storage.exit(() => seen.push(String(storage.getStore())));
    seen.push(await storage.run('inner', async () => { await null; return storage.getStore(); }));
    seen.push(storage.getStore());
  });
  seen.push(String(storage.getStore()), resource.runInAsyncScope(() => storage.getStore()));
  storage.enterWith('entered');
  await null;
  seen.push(storage.getStore());
  storage.disable();
  seen.push(String(storage.getStore()));
  console.log(seen.join(','));
}
main();
"#,
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    // As Node prints it.
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "outer,outer,outer,tick outer,io outer,undefined,inner,outer,undefined,bound,entered,undefined\n",
        "{executed:?}"
    );
}

#[test]
fn generated_electron_apps_show_tray_icons_and_register_global_shortcuts() {
    let fixture = app("import { app, globalShortcut, nativeImage, Notification, protocol, Tray } from 'electron';
protocol.registerSchemesAsPrivileged([{ scheme: 'demo', privileges: { secure: true, standard: true } }]);
app.whenReady().then(() => {
  const tray = new Tray(nativeImage.createFromPath('missing.png'));
  tray.setToolTip('NativeBoat test');
  tray.setContextMenu(null);
  const bounds = tray.getBounds();
  const registered = globalShortcut.register('Ctrl+Alt+Shift+F24', () => {});
  console.log(typeof bounds.width, registered, globalShortcut.isRegistered('ctrl+alt+shift+f24'),
    globalShortcut.register('Ctrl+Alt+Shift+F24', () => {}), tray.isDestroyed());
  globalShortcut.unregisterAll();
  tray.destroy();
  const notification = new Notification({ title: 'Title', body: 'Body', silent: true });
  console.log(tray.isDestroyed(), globalShortcut.isRegistered('Ctrl+Alt+Shift+F24'),
    Notification.isSupported(), notification.title, notification.body, typeof notification.on);
  app.quit();
});
");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "number true true false false\ntrue false true Title Body function\n",
        "{executed:?}"
    );
}

#[test]
fn computed_requires_warn_and_throw_when_they_run() {
    let fixture =
        app("import load from './loader.cjs'; console.log(load('plugin-' + process.pid));");
    fixture.write(
        "loader.cjs",
        b"module.exports = (name) => { try { return require(name); } catch (error) { return error.code; } };",
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        err.contains("loader.cjs:1:")
            && err.contains("warning: ")
            && err.contains("MODULE_NOT_FOUND"),
        "{err}"
    );
    let executed = Command::new(fixture.path("native-output/native-test.exe"))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "MODULE_NOT_FOUND
",
        "{executed:?}"
    );
}

/// The Node.js built-ins VS Code and undici load: `util/types`,
/// `diagnostics_channel`, `perf_hooks`, `console`, `v8`, `sqlite`, `dns` and
/// `module`, with resolve hooks run by a `require` made at run time and
/// `dns.lookup` through the system resolver.
#[test]
fn compiles_and_runs_node_builtins_vs_code_loads() {
    let fixture = app(r#"import { isUint8Array } from 'node:util/types';
import diagnostics_channel from 'node:diagnostics_channel';
import { performance, PerformanceObserver, monitorEventLoopDelay } from 'node:perf_hooks';
import { Console } from 'node:console';
import { getHeapCodeStatistics, setFlagsFromString } from 'node:v8';
import { DatabaseSync } from 'node:sqlite';
import dns from 'node:dns';
import Module, { createRequire, isBuiltin, registerHooks, builtinModules } from 'node:module';
import vm from 'node:vm';
import * as inspector from 'node:inspector';
const out = [];
const failure = (run) => { try { run(); return 'none'; } catch (error) { return error.code; } };
out.push(vm.isContext(vm.createContext()), failure(() => vm.compileFunction('return 1')), inspector.url(), failure(() => inspector.open(0)));
out.push(isUint8Array(new Uint8Array(1)), isUint8Array([]));
const channel = diagnostics_channel.channel('demo');
let seen;
channel.subscribe((message) => { seen = message.x; });
channel.publish({ x: 7 });
out.push(seen, channel.hasSubscribers);
performance.mark('a');
out.push(typeof performance.now(), PerformanceObserver.supportedEntryTypes.join('+'), monitorEventLoopDelay().count);
const lines = [];
const custom = new Console({ stdout: { write: (text) => lines.push(text) } });
custom.log('x %d', 5);
out.push(JSON.stringify(lines));
out.push(getHeapCodeStatistics().code_and_metadata_size);
setFlagsFromString('--expose-gc');
try { new DatabaseSync(':memory:'); } catch (error) { out.push(error.code); }
out.push(isBuiltin('fs'), isBuiltin('node:sqlite'), isBuiltin('sqlite'), builtinModules.includes('v8'));
let conditions;
const handle = registerHooks({ resolve(specifier, context, next) { conditions = context.conditions.join(','); return next(specifier, context); } });
out.push(createRequire(import.meta.url).resolve('node:fs'), conditions);
handle.deregister();
out.push(Module._nodeModulePaths('C:/a/node_modules/b').join(';'));
out.push(typeof createRequire(import.meta.url)('path').join);
out.push(typeof process.uptime(), process.uptime() >= 0, process.constrainedMemory(), process.availableMemory() > 0, process.release.name, process.release.lts, Object.isFrozen(process.config), process.config.variables.target_arch, process.stdout.destroyed);
const drained = () => {};
process.stdout.once('drain', drained);
process.stdout.removeListener('drain', drained);
out.push(process.stdout.listenerCount('drain'));
dns.lookup('localhost', { all: true }, (error, list) => {
  out.push(error ? error.code : list.length > 0);
  dns.lookup('127.0.0.1', (failure, address, family) => {
    out.push(address, family);
    console.log(out.join('|'));
  });
});
"#);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        r#"true|ERR_FEATURE_UNAVAILABLE_ON_PLATFORM||ERR_INSPECTOR_NOT_AVAILABLE|true|false|7|true|number|mark+measure|0|["x 5\n"]|0|ERR_FEATURE_UNAVAILABLE_ON_PLATFORM|true|true|false|true|node:fs|require,node,node-addons|C:\a\node_modules\b\node_modules;C:\a\node_modules;C:\node_modules|function|number|true|0|true|node|Jod|true|x64|false|0|true|127.0.0.1|4
"#
    );
}

/// Electron's `utilityProcess.fork` starts the program's own executable as
/// the forked module, with `process.parentPort` connected: messages keep
/// structured-clone types and cycles, and a transferred `MessagePortMain`
/// carries messages between the processes. Piped standard streams exist as
/// soon as `fork` returns and carry what the child prints.
#[test]
fn compiles_and_runs_forked_utility_processes() {
    let fixture = app(
        r#"import { app, utilityProcess, MessageChannelMain } from 'electron';
import { fileURLToPath } from 'node:url';
const out = [];
const child = utilityProcess.fork(fileURLToPath(new URL('./worker.js', import.meta.url)), ['--flag'], { stdio: 'pipe' });
out.push('streams', child.stdout !== null && child.stderr !== null);
let printed = '';
child.stdout.on('data', (chunk) => { printed += chunk; });
const { port1, port2 } = new MessageChannelMain();
child.on('spawn', () => out.push('spawn', typeof child.pid));
child.on('message', (message) => {
  out.push(JSON.stringify(message));
  port1.postMessage('ping');
});
port1.on('message', (event) => {
  out.push(JSON.stringify(event.data));
  child.kill();
});
port1.start();
child.on('exit', () => {
  const finish = () => {
    out.push(JSON.stringify(printed.trim()));
    console.log(out.join('|'));
    app.quit();
  };
  if (child.stdout.readableEnded) finish();
  else child.stdout.on('end', finish);
});
const data = { list: [1, 'a', null, undefined], map: new Map([[1, 2]]), when: new Date(0), bytes: new Uint8Array([3, 4]), big: 5n };
data.self = data;
child.postMessage(data, [port2]);
setTimeout(() => {
  console.log('timeout', out.join('|'));
  app.quit();
}, 20000);
"#,
    );
    fixture.write(
        "worker.js",
        br#"process.parentPort.on('message', (event) => {
  const { data, ports: [port] } = event;
  process.stdout.write('child output\n');
  port.on('message', (message) => port.postMessage({ echo: message.data, size: data.map.get(1) }));
  port.start();
  process.parentPort.postMessage({
    args: process.argv.slice(1),
    list: data.list.map(String),
    cyclic: data.self === data,
    date: data.when instanceof Date && data.when.getTime(),
    bytes: data.bytes instanceof Uint8Array && Array.from(data.bytes),
    big: typeof data.big,
    env: process.env.NATIVEBOAT_FORK_PORT === undefined,
  });
});
"#,
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        r#"streams|true|spawn|number|{"args":["--flag"],"list":["1","a","null","undefined"],"cyclic":true,"date":0,"bytes":[3,4],"big":"bigint","env":true}|{"echo":"ping","size":2}|"child output"
"#
    );
}

/// `using` and `await using` dispose their resources in reverse order when
/// the block ends, however it ends: errors from disposal combine with the
/// block's in a SuppressedError, `await using` awaits async disposal, and a
/// value without a dispose method throws a TypeError.
#[test]
fn compiles_and_runs_using_declarations() {
    let fixture = app(r#"const log: string[] = [];
const resource = (name: string, fail = false) => ({
  [Symbol.dispose]() {
    log.push(`dispose ${name}`);
    if (fail) throw new Error(`dispose ${name} failed`);
  },
});
function plain() {
  using a = resource('a');
  using b = resource('b');
  log.push('body');
  return 'returned';
}
log.push(plain());
for (const name of ['x', 'y']) {
  using item = resource(name);
  if (name === 'x') continue;
  log.push(`loop ${name}`);
}
try {
  using c = resource('c', true);
  throw new Error('body failed');
} catch (error: any) {
  log.push(`${error.name}: ${error.error.message} / ${error.suppressed.message}`);
}
try {
  using bad = 5 as any;
} catch (error: any) {
  log.push(error.constructor.name);
}
async function later() {
  await using d = { async [Symbol.asyncDispose]() { await null; log.push('async d'); } };
  await using e = resource('e');
  await using none = null;
  log.push('async body');
}
later().then(() => console.log(log.join('|')));
"#);
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "body|dispose b|dispose a|returned|dispose x|loop y|dispose y|dispose c|SuppressedError: dispose c failed / body failed|TypeError|async body|dispose e|async d\n"
    );
}

/// Node's `child_process.fork` starts the program's own executable as the
/// forked module with Node's IPC channel: JSON messages both ways,
/// `process.send` and `message` events in the child, `disconnect`, and the
/// child's exit code. Electron's tracing and screen sources answer as a
/// tracing service with no events and a capturer without thumbnails do.
#[test]
fn compiles_and_runs_forked_node_children_and_tracing() {
    let fixture = app(
        r#"import { app, contentTracing, desktopCapturer } from 'electron';
import { fork } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
const out = [];
const child = fork(join(import.meta.dirname, 'worker.js'), ['--one'], { silent: true });
child.on('message', (message) => {
  out.push(JSON.stringify(message));
  if (message.step === 1) child.send({ step: 2, when: new Date(0) });
});
child.on('disconnect', () => out.push('disconnect'));
child.on('exit', async (code) => {
  out.push(`exit ${code} ${child.connected}`);
  await contentTracing.startRecording({ included_categories: ['*'] });
  const file = await contentTracing.stopRecording(join(import.meta.dirname, 'trace.json'));
  out.push(readFileSync(file, 'utf8'));
  const sources = await desktopCapturer.getSources({ types: ['screen'] });
  out.push(sources.length > 0 && sources[0].id.startsWith('screen:') && sources[0].thumbnail.isEmpty());
  const missing = fork(join(import.meta.dirname, 'nowhere.js'));
  missing.on('exit', (missingCode) => {
    out.push(`missing ${missingCode}`);
    console.log(out.join('|'));
    app.quit();
  });
});
child.send({ step: 0, skip: undefined });
setTimeout(() => {
  console.log('timeout', out.join('|'));
  app.quit();
}, 20000);
"#,
    );
    fixture.write(
        "worker.js",
        br#"process.on('message', (message) => {
  if (message.step === 0) {
    process.send({ step: 1, args: process.argv.slice(2), script: process.argv[1].endsWith('worker.js'), keys: Object.keys(message), connected: process.connected });
  } else {
    process.send({ step: 3, when: message.when }, () => process.disconnect());
  }
});
process.on('disconnect', () => process.exit(7));
"#,
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        r#"{"step":1,"args":["--one"],"script":true,"keys":["step"],"connected":true}|{"step":3,"when":"1970-01-01T00:00:00.000Z"}|disconnect|exit 7 false|{"traceEvents":[],"metadata":{}}|true|missing 1
"#
    );
}

/// Native addons the runtime provides load from their packages' requests,
/// by relative path or through `bindings`, and run on the event loop: a
/// registry read, an spdlog file, the process list and a parcel watcher
/// that sees a new file.
#[test]
fn compiles_and_runs_runtime_native_addons() {
    let fixture = app(r#"import registry from '@vscode/windows-registry';
import spdlog from '@vscode/spdlog';
import watcher from '@parcel/watcher';
import tree from '@vscode/windows-process-tree';
import { spawn } from 'node-pty';
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
const out = [];
out.push(typeof registry.GetStringRegKey('HKEY_LOCAL_MACHINE', 'SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion', 'ProductName'));
const dir = mkdtempSync(join(tmpdir(), 'nb-addons-'));
const logger = new spdlog.Logger('rotating', 'test', join(dir, 'main.log'), 1 << 20, 2);
logger.clearFormatters();
logger.info('hello');
logger.debug('hidden');
logger.drop();
out.push(readFileSync(join(dir, 'main.log'), 'utf8'));
const seen = (error, events) => {
  out.push(error, JSON.stringify(events.map((event) => [event.path.slice(dir.length), event.type])));
  watcher.unsubscribe(dir, seen, {}).then(() => {
    rmSync(dir, { recursive: true, force: true });
    let text = '';
    const pty = spawn('cmd.exe', ['/c', 'echo native-pty'], { cols: 100, rows: 30 });
    pty.onData((data) => { text += data; });
    pty.onExit(({ exitCode }) => {
      out.push(typeof pty.pid, pty.cols, text.includes('native-pty'), exitCode);
      console.log(out.join('|'));
    });
  });
};
tree.getProcessList((list) => {
  out.push(list.some((entry) => entry.pid === process.pid && typeof entry.name === 'string'));
  watcher.subscribe(dir, seen, { ignorePaths: [join(dir, 'main.log')] }).then(() => writeFileSync(join(dir, 'new.txt'), 'x'));
}, 0);
setTimeout(() => console.log('timeout', out.join('|')), 10000).unref();
"#);
    for (file, source) in [
        (
            "node_modules/@vscode/windows-registry/index.js",
            "module.exports = require('./build/Release/winregistry.node');",
        ),
        (
            "node_modules/@vscode/spdlog/index.js",
            "module.exports = require('bindings')('spdlog');",
        ),
        (
            "node_modules/@parcel/watcher/index.js",
            "module.exports = require('./build/Release/watcher.node');",
        ),
        (
            "node_modules/@vscode/windows-process-tree/index.js",
            "module.exports = require('./build/Release/windows_process_tree.node');",
        ),
    ] {
        fixture.write(file, source.as_bytes());
        // CommonJS packages, as the real ones are.
        let package = std::path::Path::new(file)
            .parent()
            .unwrap()
            .join("package.json");
        fixture.write(&package.to_string_lossy(), br#"{"main":"index.js"}"#);
    }
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = output_within(
        Command::new(&executable).env("PATH", ""),
        std::time::Duration::from_secs(60),
    );
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        r#"string|hello|true||[["\\new.txt","create"]]|number|100|true|0
"#
    );
}

/// A page compiles the package scripts its code names by package and file,
/// and loads one, as a classic script, when it imports its URL; a URL no
/// compiled script ends rejects with `ERR_MODULE_NOT_FOUND`. A file the page
/// also imports stays a module there.
#[test]
fn pages_load_the_package_scripts_they_name() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-scripts-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, ipcMain } from 'electron';\n\
             ipcMain.on('seen', (_event, value) => { console.log('seen', value); app.quit(); });\n\
             setTimeout(() => { console.log('lost'); app.quit(); }, 5000);\n\
             new BrowserWindow({ show: false, webPreferences: { preload: './preload.ts' } }).loadFile('index.html');",
        ),
        (
            "index.html",
            "<!doctype html><html><head><script type=module src='./page.js'></script></head><body></body></html>",
        ),
        (
            "preload.ts",
            "import { contextBridge, ipcRenderer } from 'electron';\n\
             contextBridge.exposeInMainWorld('api', { send: (channel, value) => ipcRenderer.send(channel, value) });",
        ),
        (
            "page.js",
            "import cjs from './node_modules/cjs-sample/lib.js';\n\
             const before = globalThis.count;\n\
             const load = (pkg, file) => import(['.', 'node_modules', pkg, file].join('/'));\n\
             load('umd-sample', 'dist/main.js')\n\
               .then(() => load('esm-sample', 'dist/index.js'))\n\
               .then((esm) => load('nothing-here', 'x.js').then(() => 'unexpected', (error) => `${esm.value} ${error.code}`))\n\
               .then((code) => load('cjs-sample', 'lib.js').then(() => code))\n\
               .then((code) => api.send('seen', `${globalThis.umdValue} ${code} ${cjs.count} ${before} ${globalThis.count}`));",
        ),
        (
            "node_modules/cjs-sample/package.json",
            r#"{"name":"cjs-sample","version":"1.0.0"}"#,
        ),
        (
            "node_modules/cjs-sample/lib.js",
            "var count = 1; if (typeof module == 'object') module.exports = { count: count };",
        ),
        (
            "node_modules/umd-sample/package.json",
            r#"{"name":"umd-sample","version":"1.0.0"}"#,
        ),
        (
            "node_modules/umd-sample/dist/main.js",
            "var umdValue = 40 + 2;",
        ),
        (
            "node_modules/esm-sample/package.json",
            r#"{"name":"esm-sample","version":"1.0.0","type":"module"}"#,
        ),
        (
            "node_modules/esm-sample/dist/index.js",
            "const seven = 7; export { seven as value };",
        ),
    ]);
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executed = Command::new(fixture.path("native-output/native-scripts-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "seen 42 7 ERR_MODULE_NOT_FOUND 1 undefined 1\n"
    );
}

/// Package scripts a page inserts at once run one at a time, each followed
/// by its `load` event, so an AMD loader that takes the `define` call a
/// script made when the script loads pairs each script with its own, as
/// VS Code's loads `vscode-textmate` and `vscode-oniguruma`.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn inserted_scripts_get_their_load_event_before_the_next_runs() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let umd = |name: &str| {
        format!(
            "!function(t,n){{\"object\"==typeof exports&&\"object\"==typeof module?module.exports=n():\"function\"==typeof define&&define.amd?define([],n):t.{name}=n()}}(this,()=>({{name:'{name}'}}));"
        )
    };
    let (one, two) = (umd("one"), umd("two"));
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-amd-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, ipcMain } from 'electron';\n\
             ipcMain.on('seen', (_event, value) => { console.log('seen', value); app.quit(); });\n\
             setTimeout(() => { console.log('lost'); app.quit(); }, 5000);\n\
             new BrowserWindow({ show: false, webPreferences: { preload: './preload.ts' } }).loadFile('index.html');",
        ),
        (
            "index.html",
            "<!doctype html><html><head><script type=module src='./page.js'></script></head><body></body></html>",
        ),
        (
            "preload.ts",
            "import { contextBridge, ipcRenderer } from 'electron';\n\
             contextBridge.exposeInMainWorld('api', { send: (channel, value) => ipcRenderer.send(channel, value) });",
        ),
        (
            "page.js",
            "const calls = [];\n\
             globalThis.define = (dependencies, callback) => calls.push(callback);\n\
             globalThis.define.amd = true;\n\
             const load = (pkg, file) => new Promise((resolve) => {\n\
               const script = document.createElement('script');\n\
               script.addEventListener('load', () => { const call = calls.pop(); resolve(call ? call() : { name: `no-define(${typeof globalThis.one})` }); });\n\
               script.addEventListener('error', () => resolve({ name: `error(${file})` }));\n\
               script.setAttribute('src', ['.', 'node_modules', pkg, file].join('/'));\n\
               document.head.appendChild(script);\n\
             });\n\
             Promise.all([load('amd-one', 'main.js'), load('amd-two', 'main.js')])\n\
               .then(([first, second]) => api.send('seen', `${first.name} ${second.name}`));",
        ),
        (
            "node_modules/amd-one/package.json",
            r#"{"name":"amd-one","version":"1.0.0"}"#,
        ),
        ("node_modules/amd-one/main.js", one.as_str()),
        (
            "node_modules/amd-two/package.json",
            r#"{"name":"amd-two","version":"1.0.0"}"#,
        ),
        ("node_modules/amd-two/main.js", two.as_str()),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executed = Command::new(fixture.path("native-output/native-amd-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "seen one two\n",
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}

/// A script a page builds from a template literal and inserts runs as the
/// template compiled it: a substitution in a string is that string's text,
/// one standing for an expression is evaluated, as VS Code's webview host
/// builds the script that defines `acquireVsCodeApi`.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn inserted_scripts_built_from_templates_run() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-template-test","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, ipcMain } from 'electron';\n\
             ipcMain.on('seen', (_event, value) => { console.log('seen', value); app.quit(); });\n\
             setTimeout(() => { console.log('lost'); app.quit(); }, 5000);\n\
             new BrowserWindow({ show: false, webPreferences: { preload: './preload.ts' } }).loadFile('index.html');",
        ),
        (
            "index.html",
            "<!doctype html><html><head><script type=module src='./page.js'></script></head><body></body></html>",
        ),
        (
            "preload.ts",
            "import { contextBridge, ipcRenderer } from 'electron';\n\
             contextBridge.exposeInMainWorld('api', { send: (channel, value) => ipcRenderer.send(channel, value) });",
        ),
        (
            "page.js",
            r#"const build = (name, state, many) => `
  globalThis.templated = (function () {
    const key = '${name}';
    let state = ${state};
    let calls = 0;
    return () => { calls += 1; if (calls > 1 && !${many}) throw new Error('once'); return key + ':' + JSON.stringify(state); };
  })();
`;
const script = document.createElement('script');
script.textContent = build('api', 'JSON.parse(decodeURIComponent("%7B%22n%22%3A1%7D"))', true);
document.head.appendChild(script);
api.send('seen', `${globalThis.templated()} ${globalThis.templated()}`);"#,
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executed = Command::new(fixture.path("native-output/native-template-test.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "seen api:{\"n\":1} api:{\"n\":1}\n",
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}

/// `@vscode/sqlite3`'s binding over the runtime's engine, as VS Code's
/// storage drives it: open, `exec`, a prepared upsert with a Buffer value,
/// `all`, an error with SQLite's code, close, and the file opened again.
#[test]
fn compiles_and_runs_sqlite_databases() {
    let fixture = app(r#"import sqlite3 from '@vscode/sqlite3';
import { mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Buffer } from 'node:buffer';
const dir = mkdtempSync(join(tmpdir(), 'nb-sqlite-'));
const file = join(dir, 'state.vscdb');
const out = [];
const db = new sqlite3.Database(file, (error) => {
  out.push('open', error, db.open);
  db.exec('PRAGMA user_version = 1; CREATE TABLE IF NOT EXISTS ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)', (error) => {
    out.push('exec', error);
    const insert = new sqlite3.Statement(db, 'INSERT INTO ItemTable VALUES (?,?),(?,?) ON CONFLICT (key) DO UPDATE SET value = excluded.value WHERE value != excluded.value');
    insert.run(['a', 'one', 'b', Buffer.from('two')], function (error) {
      out.push('run', error, this.changes);
      insert.finalize();
      new sqlite3.Statement(db, 'SELECT * FROM ItemTable ORDER BY key').all((error, rows) => {
        out.push('all', error, JSON.stringify(rows.map((row) => [row.key, typeof row.value === 'string' ? row.value : 'buffer:' + row.value.toString()])));
        new sqlite3.Statement(db, 'SELECT nope FROM ItemTable').get((error) => {
          out.push('error', error.code, error.errno);
          db.close(() => {
            const again = new sqlite3.Database(file, () => {
              new sqlite3.Statement(again, 'SELECT count(*) AS n FROM ItemTable').get((error, row) => {
                out.push('reopened', row.n);
                again.close(() => {
                  rmSync(dir, { recursive: true, force: true });
                  console.log(out.join('|'));
                });
              });
            });
          });
        });
      });
    });
  });
});
"#);
    fixture.write(
        "node_modules/@vscode/sqlite3/index.js",
        b"module.exports = require('./build/Release/vscode-sqlite3.node');",
    );
    fixture.write(
        "node_modules/@vscode/sqlite3/package.json",
        br#"{"main":"index.js"}"#,
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        r#"open||true|exec||run||2|all||[["a","one"],["b","buffer:two"]]|error|SQLITE_ERROR|1|reopened|2
"#
    );
}

/// The WebAssembly JavaScript API over the runtime's engine, compared with
/// Node's output: a module importing a JavaScript function, its exported
/// functions with i32 and i64 values, its memory's buffer before and after
/// growing, a trap, the module's reflection, compile and link errors,
/// globals and tables.
#[test]
fn compiles_and_runs_webassembly_modules() {
    let fixture = app(
        r#"const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 24, 5, 96, 2, 127, 127, 1, 127, 96, 1, 126, 1, 126, 96, 1, 127, 0, 96, 0, 0, 96, 1, 127, 1, 127, 2, 11, 1, 3, 101, 110, 118, 3, 108, 111, 103, 0, 2, 3, 6, 5, 0, 1, 2, 3, 4, 5, 3, 1, 0, 1, 7, 44, 6, 3, 97, 100, 100, 0, 1, 3, 102, 97, 99, 0, 2, 5, 115, 104, 111, 117, 116, 0, 3, 4, 98, 111, 111, 109, 0, 4, 4, 112, 101, 101, 107, 0, 5, 6, 109, 101, 109, 111, 114, 121, 2, 0, 10, 53, 5, 7, 0, 32, 0, 32, 1, 106, 11, 21, 0, 32, 0, 80, 4, 126, 66, 1, 5, 32, 0, 32, 0, 66, 1, 125, 16, 2, 126, 11, 11, 9, 0, 32, 0, 65, 1, 106, 16, 0, 11, 3, 0, 0, 11, 7, 0, 32, 0, 45, 0, 0, 11, 11, 8, 1, 0, 65, 8, 11, 2, 104, 105]);
const out = [typeof WebAssembly, WebAssembly.validate(bytes), WebAssembly.validate(new Uint8Array([1, 2, 3]))];
const logged = [];
WebAssembly.instantiate(bytes, { env: { log: (value) => logged.push(value) } }).then(({ module, instance }) => {
  const { add, fac, shout, boom, peek, memory } = instance.exports;
  out.push(module instanceof WebAssembly.Module, instance instanceof WebAssembly.Instance);
  out.push(add(2, 3), add(0x7fffffff, 1), String(fac(20n)), add.length, add.name);
  shout(41);
  out.push(logged.join());
  out.push(memory instanceof WebAssembly.Memory, memory.buffer.byteLength, peek(8), peek(9));
  const before = memory.buffer;
  new Uint8Array(before)[100] = 200;
  out.push(peek(100), memory.grow(2), memory.buffer.byteLength, memory.buffer === before);
  try {
    boom();
  } catch (error) {
    out.push(error instanceof WebAssembly.RuntimeError, error.name, error.message);
  }
  try {
    fac(1);
  } catch (error) {
    out.push(error.name);
  }
  out.push(WebAssembly.Module.exports(module).map((entry) => entry.name + ':' + entry.kind).join());
  out.push(WebAssembly.Module.imports(module).map((entry) => entry.module + '.' + entry.name + ':' + entry.kind).join());
  try {
    new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 2, 0, 0, 0]));
  } catch (error) {
    out.push(error.name, error.message);
  }
  try {
    new WebAssembly.Instance(module, { env: { log: 1 } });
  } catch (error) {
    out.push(error.name, error.message);
  }
  const global = new WebAssembly.Global({ value: 'i32', mutable: true }, 5);
  global.value = 6.9;
  out.push(global.value);
  const table = new WebAssembly.Table({ element: 'anyfunc', initial: 2 });
  table.set(0, add);
  out.push(table.get(0) === add, table.get(1), table.length, table.grow(1), table.length);
  out.push(Object.prototype.toString.call(WebAssembly), Object.isFrozen(instance.exports));
  console.log(out.join('|'));
});
"#,
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        r#"object|true|false|true|true|5|-2147483648|2432902008176640000|2|1|42|true|65536|104|105|200|1|196608|false|true|RuntimeError|unreachable|TypeError|add:function,fac:function,shout:function,boom:function,peek:function,memory:memory|env.log:function|CompileError|WebAssembly.Module(): expected version 01 00 00 00, found 02 00 00 00 @+4|LinkError|WebAssembly.Instance(): Import #0 module="env" function="log": function import requires a callable|6|true||2|2|3|[object WebAssembly]|true
"#
    );
}

/// Node's message ports within one process, compared with Node's output:
/// the MessageChannel, MessagePort, MessageEvent and BroadcastChannel
/// globals and worker_threads exports, messages waiting until a port starts
/// and taken by receiveMessageOnPort, clone errors, Node-style and event
/// listeners with microtasks between messages, a port moved inside a
/// message, closing both ends after their messages, and BroadcastChannels
/// reaching only the others of their name. The program ends once its ports
/// and channels close.
#[test]
fn compiles_and_runs_node_message_ports() {
    let fixture = app(
        r#"import { MessageChannel as WorkerChannel, receiveMessageOnPort, isMainThread, parentPort, BroadcastChannel as WorkerBroadcast } from 'worker_threads';
const out = [];
out.push([typeof MessageChannel, typeof MessagePort, typeof MessageEvent, typeof BroadcastChannel].join());
out.push(WorkerChannel === MessageChannel, WorkerBroadcast === BroadcastChannel, isMainThread, parentPort);
try {
  new MessagePort();
} catch (error) {
  out.push(error.name);
}
const event = new MessageEvent('message', { data: { a: 1 }, origin: 'x' });
out.push(event instanceof Event, event.type, JSON.stringify(event.data), event.origin, event.lastEventId, event.ports.length);

const { port1, port2 } = new MessageChannel();
out.push(port1 instanceof MessagePort, port1 instanceof EventTarget);
const sent = { list: [1, 2], when: new Date(0) };
port1.postMessage(sent);
port1.postMessage('second');
const waiting = receiveMessageOnPort(port2);
out.push(JSON.stringify(waiting), waiting.message !== sent, receiveMessageOnPort(port2).message, receiveMessageOnPort(port2));
try {
  port1.postMessage(() => {});
} catch (error) {
  out.push(error.name, error instanceof DOMException);
}
try {
  port1.postMessage(1, [port1]);
} catch (error) {
  out.push(error.name, error.message);
}
try {
  port1.postMessage(1, [{}]);
} catch (error) {
  out.push(error.name);
}

const log = [];
const broadcast = [];
port2.on('message', (message) => {
  log.push('on:' + JSON.stringify(message));
  Promise.resolve().then(() => log.push('microtask'));
});
port2.addEventListener('message', (message) => log.push('event:' + (message instanceof MessageEvent)));
out.push(port2.hasRef());
port1.postMessage({ n: 1 });
port1.postMessage({ n: 2 });

const other = new MessageChannel();
port1.postMessage({ port: other.port2, n: 3 }, [other.port2]);
try {
  other.port2.postMessage('moved');
  out.push('moved port ignores');
} catch (error) {
  out.push(error.name);
}

const channelA = new BroadcastChannel('room');
const channelB = new BroadcastChannel('room');
const channelC = new BroadcastChannel('elsewhere');
channelA.onmessage = (message) => broadcast.push('A:' + message.data);
channelB.onmessage = (message) => broadcast.push('B:' + (message instanceof MessageEvent) + ':' + message.data);
channelC.onmessage = (message) => broadcast.push('C:' + message.data);
channelA.postMessage('hello');
out.push(channelA.name);

port2.onmessage = (message) => {
  if (message.data && message.data.port) {
    const arrived = message.data.port;
    log.push('arrived:' + (arrived instanceof MessagePort) + ':' + (message.ports[0] === arrived) + ':' + (arrived !== other.port2));
    arrived.on('message', (value) => {
      log.push('through:' + value);
      arrived.close();
    });
    other.port1.postMessage('via moved');
    other.port1.on('close', () => {
      log.push('other closed');
      port1.close();
    });
  }
};
port2.on('close', () => {
  log.push('port2 closed');
  out.push(port2.hasRef());
  channelA.close();
  channelB.close();
  channelC.close();
  try {
    channelA.postMessage('late');
  } catch (error) {
    out.push(error.name);
  }
  setTimeout(() => {
    console.log(out.join('|'));
    console.log(log.join('|'));
    console.log(broadcast.join('|'));
  }, 10);
});
"#,
    );
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        r#"function,function,function,function|true|true|true||TypeError|true|message|{"a":1}|x||0|true|true|{"message":{"list":[1,2],"when":"1970-01-01T00:00:00.000Z"}}|true|second||DataCloneError|true|DataCloneError|Transfer list contains source port|DataCloneError|true|moved port ignores|room|false|InvalidStateError
on:{"n":1}|event:true|microtask|on:{"n":2}|event:true|microtask|on:{"port":{},"n":3}|event:true|arrived:true:true:true|microtask|through:via moved|other closed|port2 closed
B:true:hello
"#
    );
}

/// A failed job stops new jobs, unless the build keeps going: then jobs
/// independent of the failure still run, its dependents are skipped, and
/// every error is reported.
#[test]
fn dependency_jobs_stop_or_keep_going_after_failures() {
    use std::collections::BTreeSet;
    // 0 fails; 1 depends on 0; 2 and 3 are independent, 3 fails too.
    let dependencies = vec![
        BTreeSet::new(),
        BTreeSet::from([0]),
        BTreeSet::new(),
        BTreeSet::new(),
    ];
    let order = [0, 1, 2, 3];
    let ran = std::sync::Mutex::new(Vec::new());
    let work = |job: usize, _: &[Option<usize>]| {
        ran.lock().unwrap().push(job);
        if job == 0 || job == 3 {
            Err(format!("job {job} failed"))
        } else {
            Ok(job)
        }
    };
    let stopped = run_dependency_jobs(&order, &dependencies, 1, false, work).unwrap_err();
    assert_eq!(stopped, "job 0 failed");
    assert_eq!(*ran.lock().unwrap(), vec![0]);

    ran.lock().unwrap().clear();
    let all = run_dependency_jobs(&order, &dependencies, 1, true, work).unwrap_err();
    assert_eq!(
        all,
        "job 0 failed\n\njob 3 failed\n\n2 jobs failed; 1 jobs depending on them did not run"
    );
    assert_eq!(*ran.lock().unwrap(), vec![0, 2, 3]);

    let fine = run_dependency_jobs(&[1, 0, 2], &dependencies[..3], 2, true, |job, _| {
        Ok(job * 10)
    });
    assert_eq!(fine.unwrap(), vec![0, 10, 20]);
}

/// A command line past Windows' limit runs rustc with an argument file,
/// which the toolchain reads and the build removes afterwards.
#[test]
fn long_toolchain_command_lines_pass_an_argument_file() {
    let fixture = Fixture::new(&[("source/lib.rs", "pub fn answer() -> u32 { 42 }")]);
    let directory = fixture.path("source");
    let short = [OsString::from("--version")];
    assert!(argument_file(&short, &directory).is_none());
    let broken = [OsString::from("a\nb"), OsString::from("x".repeat(20_000))];
    assert!(argument_file(&broken, &directory).is_none());

    let rustc = Toolchain {
        rustc: PathBuf::from("rustc"),
        sysroot: None,
    };
    // About 40 000 characters of dependency folders, as a late chunk of a
    // large program names.
    let dependency = directory.display().to_string();
    let mut arguments = (0..300)
        .map(|_| OsString::from(format!("-Ldependency={dependency}{}", r"\.".repeat(40))))
        .collect::<Vec<_>>();
    arguments.extend(
        [
            "lib.rs",
            "--edition=2024",
            "--crate-type=rlib",
            "--crate-name=long_line",
        ]
        .map(OsString::from),
    );
    toolchain(&rustc, &arguments, &directory).unwrap();
    assert!(directory.join("liblong_line.rlib").is_file());
    let left = std::fs::read_dir(&directory)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().path().extension() == Some("args".as_ref()))
        .count();
    assert_eq!(left, 0);
}

/// Pages that load the same scripts, as VS Code's `workbench.html` and its
/// development copy `workbench-dev.html` do, share one compiled renderer
/// graph; each page still matches its own URL with its own document.
#[test]
fn pages_loading_the_same_scripts_share_one_renderer_graph() {
    let fixture = app("import { app } from 'electron'; app.whenReady();");
    fixture.write(
        "index.html",
        b"<html><body><p>product</p><script type=module src='renderer.ts'></script></body></html>",
    );
    fixture.write(
        "index-dev.html",
        b"<html><body><p>development</p><script type=module src='renderer.ts'></script></body></html>",
    );
    fixture.write(
        "other.html",
        b"<html><script type=module src='other.ts'></script></html>",
    );
    fixture.write("renderer.ts", b"console.log('renderer-ready');");
    fixture.write("other.ts", b"console.log('other-ready');");
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    for page in ["index.html", "index-dev.html", "other.html"] {
        project.pages.push(nativeboat_project::Entry {
            source: fixture.path(page),
            runtime_paths: vec![page.into()],
        });
    }
    let compilation = compile(&project);
    assert!(compilation.errors.is_empty(), "{:?}", compilation.errors);
    assert_eq!(compilation.renderers.len(), 2);
    assert_eq!(
        compilation.renderers[0].pages,
        [fixture.path("index.html"), fixture.path("index-dev.html")]
    );
    assert_eq!(compilation.renderers[1].pages, [fixture.path("other.html")]);
    let generated = application_source(&project, &compilation, false, false).unwrap();
    assert!(generated.contains("requested.ends_with(\"index.html\")"));
    assert!(generated.contains("requested.ends_with(\"index-dev.html\")"));
    assert_eq!(generated.matches("renderer_0::initialize").count(), 2);
    assert_eq!(generated.matches("renderer_1::initialize").count(), 1);
    assert!(!generated.contains("renderer_2::initialize"));
    assert!(generated.contains("product") && generated.contains("development"));
}

/// Renderer chunk crates are named after their first page, so adding,
/// removing or merging other pages keeps their cached builds.
#[test]
fn renderer_graph_names_follow_their_page_not_their_position() {
    let root = Path::new("C:/work/app");
    let name = graph_name(root, &root.join("src/Workbench.html"));
    assert_eq!(
        name,
        graph_name(
            Path::new("D:/other"),
            Path::new("D:/other/src/workbench.html")
        )
    );
    assert_ne!(name, graph_name(root, &root.join("src/workbench-dev.html")));
    assert_eq!(name.len(), 13);
    assert!(name.starts_with('p') && name[1..].bytes().all(|byte| byte.is_ascii_hexdigit()));
}

/// `net` servers and clients on Windows named pipes, as VS Code's main
/// process claims its instance: a second server on the name fails with
/// EADDRINUSE, a client exchanges data and its close ends the server's
/// stream, and a closed server's name no longer connects. The output is
/// Node 22's.
#[test]
fn generated_app_serves_and_connects_over_named_pipes() {
    let fixture = app("import net from 'net';
const path = `\\\\\\\\.\\\\pipe\\\\nativeboat-cli-${process.pid}`;
const server = net.createServer((socket) => {
  socket.setEncoding('utf8');
  socket.on('data', (chunk) => socket.write(`echo ${chunk}`));
  socket.on('end', () => {
    console.log('server saw end');
    socket.end();
  });
});
server.listen(path, () => {
  console.log(`listening ${server.address() === path}`);
  const second = net.createServer();
  second.on('error', (error) => {
    console.log(`taken ${error.code} ${error.syscall}`);
    const client = net.connect(path, () => client.write('ping'));
    client.setEncoding('utf8');
    client.on('data', (reply) => {
      console.log(reply);
      client.destroy();
    });
    client.on('close', () => server.close(() => {
      net.connect(path).on('error', (failure) => console.log(`missing ${failure.code} ${failure.syscall}`));
    }));
  });
  second.listen(path);
});");
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executable = fixture.path("native-output/native-test.exe");
    let executed = Command::new(&executable).env("PATH", "").output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "listening true\ntaken EADDRINUSE listen\necho ping\nserver saw end\nmissing ENOENT connect\n"
    );
}

/// A preload script without ES module syntax is CommonJS in a package of
/// type `module`, as Electron runs sandboxed preloads: it requires
/// `electron`, and reads the window's additional arguments from its
/// `process.argv`, as VS Code's preload does.
#[test]
fn preload_scripts_are_commonjs_and_see_the_window_arguments() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-script-preload","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { BrowserWindow, ipcMain } from 'electron';\n\
             ipcMain.on('native:config', (event, channel) => { event.returnValue = channel; });\n\
             const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.js', additionalArguments: ['--native-window-config=vscode:7'] } });\n\
             window.destroy();",
        ),
        (
            "preload.js",
            "(function () {\n\
               'use strict';\n\
               const { ipcRenderer } = require('electron');\n\
               const arg = process.argv.find((value) => value.indexOf('--native-window-config=') === 0);\n\
               const channel = ipcRenderer.sendSync('native:config', arg.split('=')[1]);\n\
               console.log('preload', typeof module, channel, process.argv.includes('--type=renderer'));\n\
             }());",
        ),
    ]);
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executable = fixture.path("native-output/native-script-preload.exe");
    let executed = Command::new(executable).output().unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "preload object vscode:7 true\n"
    );
}

/// A graph's generated crates are linked into one folder, made empty for
/// each build, so crates that depend on them name one search path.
#[test]
fn generated_crates_are_linked_into_one_dependency_folder() {
    let fixture = Fixture::new(&[
        ("cache/one/libfirst.rlib", "first"),
        ("cache/two/libsecond.rlib", "second"),
        ("staging/linked-renderer-p1/libstale.rlib", "stale"),
    ]);
    let staging = fixture.path("staging");
    let folder = linked_dependencies(&staging, "renderer", "p1").unwrap();
    assert!(!folder.join("libstale.rlib").exists());
    link_dependency(&folder, &fixture.path("cache/one/libfirst.rlib")).unwrap();
    link_dependency(&folder, &fixture.path("cache/two/libsecond.rlib")).unwrap();
    assert_eq!(
        std::fs::read_to_string(folder.join("libsecond.rlib")).unwrap(),
        "second"
    );
    std::fs::create_dir_all(fixture.path("cache/three")).unwrap();
    std::fs::write(fixture.path("cache/three/libfirst.rlib"), "newer").unwrap();
    link_dependency(&folder, &fixture.path("cache/three/libfirst.rlib")).unwrap();
    assert_eq!(
        std::fs::read_to_string(folder.join("libfirst.rlib")).unwrap(),
        "newer"
    );
}

/// `webContents.postMessage` moves a `MessagePortMain` to the window's
/// preload, which gets it as its `ipcRenderer` event's port and answers on
/// it, as VS Code's windows get their channels to the shared process; what
/// it posts before closing the port arrives before the close.
#[test]
fn message_ports_move_from_main_to_a_window_preload() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-preload-ports","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, MessageChannelMain } from 'electron';\n\
             const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.js' } });\n\
             const { port1, port2 } = new MessageChannelMain();\n\
             port1.on('message', (event) => console.log('main', event.data.text));\n\
             port1.on('close', () => { console.log('closed'); app.quit(); });\n\
             port1.start();\n\
             window.webContents.postMessage('port', 'nonce', [port2]);\n\
             port1.postMessage({ text: 'hello' });\n\
             setTimeout(() => { console.log('timeout'); app.quit(); }, 5000);",
        ),
        (
            "preload.js",
            "const { ipcRenderer } = require('electron');\n\
             ipcRenderer.on('port', (event, nonce) => {\n\
               const [port] = event.ports;\n\
               port.onmessage = (message) => { port.postMessage({ text: `${nonce} ${message.data.text} ${event.ports.length} ${typeof window.postMessage}` }); port.close(); };\n\
             });",
        ),
    ]);
    let (code, stdout, stderr) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let executed = Command::new(fixture.path("native-output/native-preload-ports.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap(),
        "main nonce hello 1 function\nclosed\n"
    );
}

/// A preload moves the port it got over IPC on to its page with
/// `window.postMessage`, and the page answers on it, as VS Code's
/// `acquirePort` does.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn message_ports_move_from_a_preload_to_its_page() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-page-ports","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            "import { app, BrowserWindow, ipcMain, MessageChannelMain } from 'electron';\n\
             const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.js' } });\n\
             ipcMain.on('acquire', (event, nonce) => {\n\
               const { port1, port2 } = new MessageChannelMain();\n\
               port1.on('message', (message) => console.log('main', message.data.text));\n\
               port1.on('close', () => { console.log('closed'); app.quit(); });\n\
               port1.start();\n\
               event.sender.postMessage('port', nonce, [port2]);\n\
               port1.postMessage({ text: 'hello' });\n\
             });\n\
             window.loadFile('index.html');\n\
             setTimeout(() => { console.log('timeout'); app.quit(); }, 5000);",
        ),
        (
            "preload.js",
            "const { ipcRenderer, contextBridge } = require('electron');\n\
             ipcRenderer.on('port', (event, nonce) => window.postMessage(nonce, '*', event.ports));\n\
             contextBridge.exposeInMainWorld('native', { acquire: (nonce) => ipcRenderer.send('acquire', nonce) });",
        ),
        (
            "index.html",
            "<!doctype html><html><body><script type='module' src='./renderer.js'></script></body></html>",
        ),
        (
            "renderer.js",
            "window.addEventListener('message', (event) => {\n\
               if (event.data !== 'n1' || event.source !== window) return;\n\
               const [port] = event.ports;\n\
               port.addEventListener('message', (message) => { port.postMessage({ text: `${event.data} ${message.data.text} ${port instanceof MessagePort} ${event.ports.length}` }); port.close(); });\n\
               port.start();\n\
             });\n\
             window.native.acquire('n1');",
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-page-ports.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(executed.stderr.is_empty(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "main n1 hello true 1\nclosed\n"
    );
}

/// A page's workers run their scripts in global scopes of their own and
/// exchange messages with the page: one started from a blob, whose script
/// the page made as VS Code makes its workers' bootstrap scripts, imports
/// the compiled module it names; another runs that module from its URL; a
/// third reports the error its script throws to the page, and a fourth the
/// syntax its script uses that a worker cannot run, quoting the script. A
/// fifth runs a blob script made from a function's text, as VS Code makes
/// its performance worker's: the worker runs the compiled function.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn page_workers_run_compiled_and_blob_scripts() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-workers","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            r#"import { app, BrowserWindow, ipcMain } from 'electron';
const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.js' } });
const lines: string[] = [];
ipcMain.on('log', (_event, text) => {
  lines.push(text);
  if (lines.length === 5) { console.log(lines.sort().join('\n')); app.quit(); }
});
window.loadFile('index.html');
setTimeout(() => { console.log('timeout', lines.join('|')); app.quit(); }, 10000);"#,
        ),
        (
            "preload.js",
            "const { ipcRenderer, contextBridge } = require('electron');\n\
             contextBridge.exposeInMainWorld('native', { log: (text) => ipcRenderer.send('log', text) });",
        ),
        (
            "index.html",
            "<!doctype html><html><body><script type='module' src='./renderer.js'></script></body></html>",
        ),
        (
            "renderer.js",
            r#"const url = new URL('./echo.js', import.meta.url).href;
const source = [
  '/*blob*/',
  `globalThis.label = ${JSON.stringify('blob')};`,
  "const policy = globalThis.trustedTypes?.createPolicy('p', { createScriptURL: value => value });",
  `try { await import(policy?.createScriptURL(${JSON.stringify(url)}) ?? ${JSON.stringify(url)}); } catch (error) { throw error; }`,
  "globalThis.postMessage({ type: 'ready' });",
].join('');
const blobWorker = new Worker(URL.createObjectURL(new Blob([source], { type: 'application/javascript' })), { type: 'module', name: 'b' });
blobWorker.onmessage = (event) => {
  if (event.data.type === 'ready') { blobWorker.postMessage('ping'); return; }
  window.native.log(`blob ${event.data}`);
  blobWorker.terminate();
};
const urlWorker = new Worker(new URL('./echo.js', import.meta.url), { type: 'module' });
urlWorker.onmessage = (event) => window.native.log(`url ${event.data}`);
urlWorker.postMessage('pong');
const failing = new Worker(URL.createObjectURL(new Blob(['throw new Error("boom");'])));
failing.onerror = (event) => { event.preventDefault(); window.native.log(`error ${event.message}`); };
const unparsed = new Worker(URL.createObjectURL(new Blob(['globalThis.x = 1; while (true) {}'])));
unparsed.onerror = (event) => { event.preventDefault(); window.native.log(`unparsed ${event.message}`); };
const fibSource = (function () {
  function fib(n) { return n <= 2 ? n : fib(n - 1) + fib(n - 2); }
  let total = 0;
  for (let i = 0; i < 3; i++) total += fib(10);
  self.postMessage({ value: total, scope: self.constructor.name });
}).toString();
const fibWorker = new Worker(URL.createObjectURL(new Blob([`(${fibSource})();`], { type: 'application/javascript' })));
fibWorker.onmessage = (event) => { window.native.log(`function ${event.data.value} ${event.data.scope}`); fibWorker.terminate(); };"#,
        ),
        (
            "echo.js",
            "self.onmessage = (event) => self.postMessage(`${self.label} ${event.data} ${self.constructor.name} ${typeof document} ${self.name}`);",
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-workers.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(executed.stderr.is_empty(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "blob blob ping DedicatedWorkerGlobalScope undefined b\n\
         error Uncaught Error: boom\n\
         function 267 DedicatedWorkerGlobalScope\n\
         unparsed Uncaught SyntaxError: a worker runs the scripts a program makes while it runs only when they declare, assign, call and import; this one uses 'while' at offset 18: \"globalThis.x = 1; while (true) {}\"\n\
         url undefined pong DedicatedWorkerGlobalScope undefined \n"
    );
}

/// A page's `<iframe>` shows a compiled document in a realm of its own:
/// the frame's inline script runs, the element gets `load`, the two
/// windows exchange messages with their sources and transferred ports, and
/// the frame's window takes the element's content box when it changes,
/// which the frame sees in its animation frames.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn page_frames_run_their_documents_and_exchange_messages() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-frames","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            r#"import { app, BrowserWindow, ipcMain } from 'electron';
const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.js' } });
const lines: string[] = [];
ipcMain.on('log', (_event, text) => {
  lines.push(text);
  if (lines.length === 5) { console.log(lines.sort().join('\n')); app.quit(); }
});
window.loadFile('index.html');
setTimeout(() => { console.log('timeout', lines.join('|')); app.quit(); }, 10000);"#,
        ),
        (
            "preload.js",
            "const { ipcRenderer, contextBridge } = require('electron');\n\
             contextBridge.exposeInMainWorld('native', { log: (text) => ipcRenderer.send('log', text) });",
        ),
        (
            "index.html",
            "<!doctype html><html><body><iframe id='f' src='frame.html'></iframe><script type='module' src='./renderer.js'></script></body></html>",
        ),
        (
            "renderer.js",
            r#"const frame = document.getElementById('f');
window.addEventListener('message', (event) => {
  window.native.log(`page got ${event.data} from frame ${event.source === frame.contentWindow} ${event.ports.length}`);
  if (event.ports.length) event.ports[0].postMessage('over port');
});
frame.addEventListener('load', () => {
  window.native.log(`frame load ${frame.contentWindow !== null}`);
  frame.contentWindow.postMessage('hello frame', '*');
  frame.style.width = '260px';
  frame.style.height = '120px';
  frame.contentWindow.postMessage('measure', '*');
});"#,
        ),
        (
            "frame.html",
            "<!doctype html><html><body><script>\n\
             window.addEventListener('message', (event) => {\n\
               if (event.data === 'measure') {\n\
                 const started = Date.now();\n\
                 const poll = () => (innerWidth === 260 && innerHeight === 120) || Date.now() - started > 3000\n\
                   ? parent.postMessage(`size ${innerWidth}x${innerHeight}`, '*') : requestAnimationFrame(poll);\n\
                 poll();\n\
                 return;\n\
               }\n\
               parent.postMessage(`frame got ${event.data} ${event.source === parent}`, '*');\n\
               const channel = new MessageChannel();\n\
               channel.port1.onmessage = (message) => parent.postMessage(`port ${message.data}`, '*');\n\
               parent.postMessage('with port', '*', [channel.port2]);\n\
             });\n\
             </script></body></html>",
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-frames.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "frame load true\n\
         page got frame got hello frame true from frame true 0\n\
         page got port over port from frame true 0\n\
         page got size 260x120 from frame true 0\n\
         page got with port from frame true 1\n"
    );
}

/// A frame of its page's origin is reached directly: the page writes the
/// frame's document with `open`, `write` and `close` once its first
/// document loaded, and the frame's `parent` is the page's window. The
/// documents are the `documents` setting's, served by a protocol.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn page_frames_of_the_same_origin_share_their_windows() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-same-origin","type":"module","main":"main.ts","devDependencies":{"electron":"44"},"nativeboat":{"documents":["*.html"]}}"#,
        ),
        (
            "main.ts",
            r#"import { app, BrowserWindow, ipcMain, protocol } from 'electron';
protocol.registerSchemesAsPrivileged([{ scheme: 'app', privileges: { standard: true, secure: true } }]);
const lines: string[] = [];
ipcMain.on('log', (_event, text) => {
  lines.push(text);
  if (lines.length === 2) { console.log(lines.join('\n')); app.quit(); }
});
app.whenReady().then(() => {
  protocol.handle('app', () => new Response(''));
  const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.js' } });
  window.loadURL('app://local/index.html');
});
setTimeout(() => { console.log('timeout', lines.join('|')); app.quit(); }, 10000);"#,
        ),
        (
            "preload.js",
            "const { ipcRenderer, contextBridge } = require('electron');\n\
             contextBridge.exposeInMainWorld('native', { log: (text) => ipcRenderer.send('log', text) });",
        ),
        (
            "index.html",
            "<!doctype html><html><body><iframe id='f' src='same.html'></iframe><script type='module' src='./renderer.js'></script></body></html>",
        ),
        (
            "same.html",
            "<!doctype html><html><body><p>first</p></body></html>",
        ),
        (
            "renderer.js",
            r#"const frame = document.getElementById('f');
const doc = frame.contentDocument;
const log = (text) => window.native.log(text);
log(`direct ${frame.contentWindow.location.pathname} ${doc !== null} ${frame.contentWindow.parent === window}`);
frame.contentWindow.addEventListener('DOMContentLoaded', () => {
  doc.open();
  doc.write('<p id="x">written</p>');
  doc.close();
  // The frame calls the page's listener, which reads the page's globals.
  window.native.log(`written ${doc.getElementById('x').textContent} ${doc.body.children.length}`);
}, { once: true });"#,
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-same-origin.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "direct /same.html true true\nwritten written 1\n"
    );
}

/// A page registers a service worker, which installs, activates and claims
/// it: the page's `fetch()` calls reach the worker's `fetch` event, and the
/// two exchange messages, the worker seeing the page among its clients.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn page_service_workers_answer_the_requests_of_the_pages_they_control() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-service-worker","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            r#"import { app, BrowserWindow, ipcMain, protocol } from 'electron';
protocol.registerSchemesAsPrivileged([{ scheme: 'app', privileges: { standard: true, secure: true } }]);
const lines: string[] = [];
ipcMain.on('log', (_event, text) => {
  lines.push(text);
  if (lines.length === 4) { console.log(lines.join('\n')); app.quit(); }
});
app.whenReady().then(() => {
  protocol.handle('app', () => new Response(''));
  const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.js' } });
  window.loadURL('app://local/index.html');
});
setTimeout(() => { console.log('timeout', lines.join('|')); app.quit(); }, 10000);"#,
        ),
        (
            "preload.js",
            "const { ipcRenderer, contextBridge } = require('electron');\n\
             contextBridge.exposeInMainWorld('native', { log: (text) => ipcRenderer.send('log', text) });",
        ),
        (
            "index.html",
            "<!doctype html><html><body><script type='module' src='./renderer.js'></script></body></html>",
        ),
        (
            "renderer.js",
            r#"const log = (text) => window.native.log(text);
const container = navigator.serviceWorker;
container.addEventListener('message', (event) => log(`message ${event.data} ${event.source === container.controller}`));
(async () => {
  const registration = await navigator.serviceWorker.register(`sw.js?v=${1}`);
  log(`registered ${registration.scope.endsWith('local/')} ${container.controller === null}`);
  if (!container.controller) await new Promise((resolve) => container.addEventListener('controllerchange', resolve, { once: true }));
  const response = await fetch('https://example.test/hello');
  log(`fetched ${response.status} ${await response.text()} ${container.controller.state}`);
  await new Promise((resolve) => {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = 'https://example.test/style.css';
    link.addEventListener('load', () => {
      log(`styled ${getComputedStyle(document.body).marginLeft}`);
      resolve();
    });
    document.head.append(link);
  });
  container.controller.postMessage('ping');
})().catch((error) => log(`error ${error}`));"#,
        ),
        (
            "sw.js",
            r#"self.addEventListener('install', (event) => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));
self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (url.pathname === '/style.css') event.respondWith(new Response('body { margin-left: 13px }', { headers: { 'content-type': 'text/css' } }));
  else if (url.hostname === 'example.test') event.respondWith(new Response(`from worker ${url.pathname}`, { status: 201 }));
});
self.addEventListener('message', async (event) => {
  const clients = await self.clients.matchAll({ includeUncontrolled: true });
  event.source.postMessage(`pong ${event.data} ${clients.length}`);
});"#,
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-service-worker.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "registered true true\nfetched 201 from worker /hello activated\nstyled 13px\nmessage pong ping 1 true\n"
    );
}

/// A compiled page defines custom elements: `document.createElement` and
/// `new` construct them, a definition upgrades the elements the page already
/// has, its callbacks follow insertions, moves, removals and observed
/// attributes, and `:defined` restyles an element once it is upgraded.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn page_custom_elements_upgrade_and_react() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-custom-elements","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            r#"import { app, BrowserWindow, ipcMain } from 'electron';
const window = new BrowserWindow({ show: false, webPreferences: { preload: './preload.js' } });
ipcMain.on('log', (_event, text) => { console.log(text); app.quit(); });
window.loadFile('index.html');
setTimeout(() => { console.log('timeout'); app.quit(); }, 10000);"#,
        ),
        (
            "preload.js",
            "const { ipcRenderer, contextBridge } = require('electron');\n\
             contextBridge.exposeInMainWorld('native', { log: (text) => ipcRenderer.send('log', text) });",
        ),
        (
            "index.html",
            "<!doctype html><html><head><style>x-late { display: block; } x-late:not(:defined) { display: none; }</style></head>\
             <body><x-late id=late title=first>late</x-late><script type='module' src='./renderer.js'></script></body></html>",
        ),
        (
            "renderer.js",
            r#"const log = [];
class Observer extends HTMLElement {
  connectedCallback() { log.push('connected'); this.onDidConnect?.(); }
  disconnectedCallback() { log.push('disconnected'); }
}
if (!customElements.get('connection-observer')) customElements.define('connection-observer', Observer);
const observer = document.createElement('connection-observer');
observer.onDidConnect = () => log.push('mounted');
const container = document.createElement('div');
container.appendChild(observer);
document.body.appendChild(container);
document.body.prepend(container);
container.remove();
class Counted extends HTMLElement {
  count = 2;
  constructor() { super(); log.push(`constructed ${this.localName} ${this.count}`); }
}
customElements.define('x-counted', Counted);
const counted = new Counted();
log.push(String(counted instanceof Counted && counted.matches(':defined')));
const late = document.getElementById('late');
const hidden = late.offsetWidth;
class Late extends HTMLElement {
  static observedAttributes = ['title'];
  attributeChangedCallback(name, old, value) { log.push(`${name} ${old} ${value}`); }
}
customElements.define('x-late', Late);
late.title = 'second';
log.push(`${hidden} ${late.offsetWidth > 0} ${late instanceof Late}`);
window.native.log(log.join(', '));"#,
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-custom-elements.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(executed.stderr.is_empty(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "connected, mounted, disconnected, connected, mounted, disconnected, \
         constructed x-counted 2, true, title null first, title first second, 0 true true\n"
    );
}

/// A compiled page renders a shadow tree as VS Code's context view makes
/// one: the host shows the tree with its own style sheet, `:host` and
/// `:host-context()` rules, a slotted child, and no unslotted ones; the
/// document's rules stay out, and hit testing and clicks are retargeted to
/// the host outside the tree.
#[test]
#[ignore = "requires NATIVEBOAT_RENDERER_TOOLCHAIN with packaged renderer libraries"]
fn page_shadow_trees_render_scoped_styles_and_retarget_events() {
    let sysroot = PathBuf::from(std::env::var_os("NATIVEBOAT_RENDERER_TOOLCHAIN").unwrap());
    assert!(sysroot.join("renderer-libs").is_dir());
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-shadow","type":"module","main":"main.ts","devDependencies":{"electron":"44"}}"#,
        ),
        (
            "main.ts",
            r#"import { app, BrowserWindow, ipcMain } from 'electron';
const window = new BrowserWindow({ show: false, width: 400, height: 300, webPreferences: { preload: './preload.js' } });
ipcMain.on('log', (_event, text) => { console.log(text); app.quit(); });
window.loadFile('index.html');
setTimeout(() => { console.log('timeout'); app.quit(); }, 10000);"#,
        ),
        (
            "preload.js",
            "const { ipcRenderer, contextBridge } = require('electron');\n\
             contextBridge.exposeInMainWorld('native', { log: (text) => ipcRenderer.send('log', text) });",
        ),
        (
            "index.html",
            "<!doctype html><html><head><style>body { margin: 0 } .item { color: rgb(255, 0, 0) } .shadow-root-host { color: rgb(0, 0, 255) }</style></head>\
             <body class=hc-black><div class=shadow-root-host><b slot=extra>light</b><i>hidden</i></div><script type='module' src='./renderer.js'></script></body></html>",
        ),
        (
            "renderer.js",
            r#"const host = document.querySelector('.shadow-root-host');
const root = host.attachShadow({ mode: 'open' });
const style = document.createElement('style');
style.textContent = ':host { all: initial; display: block; color: rgb(0, 128, 0) } .item { display: block; width: 120px; height: 30px; color: rgb(1, 2, 3) } :host-context(.hc-black) .item { outline: 1px solid rgb(4, 5, 6) } ::slotted(b) { display: block; height: 10px }';
root.appendChild(style);
const view = document.createElement('div');
view.className = 'item';
view.textContent = 'menu';
root.appendChild(view);
const slot = document.createElement('slot');
slot.name = 'extra';
root.appendChild(slot);
requestAnimationFrame(() => {
  const rect = view.getBoundingClientRect();
  const light = host.querySelector('b').getBoundingClientRect();
  const hidden = host.querySelector('i').getBoundingClientRect();
  const x = rect.left + 10, y = rect.top + 10;
  const clicks = [];
  document.addEventListener('click', (event) => clicks.push(`document ${event.target === host} ${event.composedPath()[0] === view}`));
  view.addEventListener('click', (event) => clicks.push(`view ${event.target === view}`));
  view.click();
  window.native.log([
    getComputedStyle(view).color,
    getComputedStyle(view).outlineColor,
    getComputedStyle(host).color,
    `${rect.width}x${rect.height}`,
    light.height,
    hidden.height,
    document.elementFromPoint(x, y) === host,
    root.elementFromPoint(x, y) === view,
    clicks.join(' / '),
  ].join(', '));
});"#,
        ),
    ]);
    let mut session = shimforge::Session::new();
    let select = shimforge::mock!(
        session,
        select_toolchain,
        fn(Option<&Path>, &Path) -> Toolchain
    );
    select.expect().returning(move |_, _| Toolchain {
        rustc: sysroot.join("bin/rustc.exe"),
        sysroot: Some(sysroot.clone()),
    });
    let (code, out, err) = run(&fixture, &["--out", "native-output"]);
    assert_eq!(code, 0, "{out}{err}");
    let executed = Command::new(fixture.path("native-output/native-shadow.exe"))
        .output()
        .unwrap();
    assert!(executed.status.success(), "{executed:?}");
    assert!(executed.stderr.is_empty(), "{executed:?}");
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "rgb(1, 2, 3), rgb(4, 5, 6), rgb(0, 0, 255), 120x30, 10, 0, true, true, \
         view true / document true true\n"
    );
}

/// Cached libraries leave out LLVM bitcode: only link-time optimization
/// reads it, and it was nearly half of every chunk's bytes.
#[test]
fn generated_libraries_leave_out_llvm_bitcode() {
    let arguments = generated_rlib_arguments(
        &[Path::new("deps")],
        &[],
        "chunk",
        "chunk.rs",
        Path::new("libchunk.rlib"),
        "s",
    );
    assert!(
        arguments
            .iter()
            .any(|argument| argument == "-Cembed-bitcode=no"),
        "{arguments:?}"
    );
}
