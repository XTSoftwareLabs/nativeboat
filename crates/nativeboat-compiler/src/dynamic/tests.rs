use super::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn lazy_project_factories_allow_unreferenced_entry_evaluators() {
    let module = link(&[
        (
            "main.ts",
            "import { value } from 'lazy.ts'; export default value;",
        ),
        ("lazy.ts", "export const value = 1;"),
    ])
    .unwrap();
    assert!(
        module
            .rust_source()
            .contains("#[allow(dead_code)]\nimpl Project")
    );
}

#[test]
fn electron_native_graph_links_only_implemented_named_exports() {
    let entries = [Path::new("main.mjs")];
    let compile = |source: &str| {
        compile_sources_with_entries(
            &entries,
            &mut |_, request, _| (request == "electron").then(|| PathBuf::from("electron")),
            &mut |path| {
                Some(if path == Path::new("main.mjs") {
                    ModuleSource {
                        text: source.to_owned(),
                        kind: ModuleKind::EsModule,
                    }
                } else {
                    ModuleSource {
                        text: "electron".to_owned(),
                        kind: ModuleKind::Native,
                    }
                })
            },
        )
    };

    let module =
        compile(
            "import { app, BrowserWindow, clipboard, contextBridge, dialog, ipcMain, ipcRenderer, Menu, MenuItem, nativeImage, nativeTheme, net, screen, session, shell, webFrame, webUtils } from 'electron'; export {app, BrowserWindow, clipboard, contextBridge, dialog, ipcMain, ipcRenderer, Menu, MenuItem, nativeImage, nativeTheme, net, screen, session, shell, webFrame, webUtils};",
        )
        .unwrap();
    assert!(module.native_dependencies().contains("electron"));
    assert!(
        module
            .rust_source()
            .contains("electron_builtin::module(realm)")
    );

    compile("import { clipboard } from 'electron'; clipboard.readText();").unwrap();
    compile("import { dialog } from 'electron'; dialog.showOpenDialog({properties:['openFile']});")
        .unwrap();
    compile("import { nativeImage } from 'electron'; nativeImage.createFromBuffer();").unwrap();
    compile("import { net, session } from 'electron'; net.fetch('https://example.com'); session.defaultSession.setProxy({mode:'system'});").unwrap();
    compile("import { autoUpdater } from 'electron'; autoUpdater.checkForUpdates();").unwrap();
    compile("import { powerMonitor, powerSaveBlocker, safeStorage, systemPreferences, webContents } from 'electron'; powerMonitor.getSystemIdleTime(); powerSaveBlocker.start('prevent-app-suspension'); safeStorage.isEncryptionAvailable(); systemPreferences.getAccentColor(); webContents.getAllWebContents();").unwrap();
    let diagnostics =
        compile("import { BaseWindow } from 'electron'; new BaseWindow();").unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("has no export 'BaseWindow'") })
    );
    // Exports a native application cannot provide compile, with a warning.
    let unavailable = compile(
        "import { WebContentsView } from 'electron'; export const make = () => new WebContentsView();",
    )
    .unwrap();
    assert!(unavailable.warnings().iter().any(|warning| {
        warning
            .message
            .contains("'WebContentsView' from 'electron' is not available")
    }));
}

#[test]
fn child_process_native_graph_only_links_implemented_exports() {
    let entries = [Path::new("main.mjs")];
    let compile = |source: &str| {
        compile_sources_with_entries(
            &entries,
            &mut |_, request, _| {
                (request == "child_process").then(|| PathBuf::from("child_process"))
            },
            &mut |path| {
                Some(if path == Path::new("main.mjs") {
                    ModuleSource {
                        text: source.to_owned(),
                        kind: ModuleKind::EsModule,
                    }
                } else {
                    ModuleSource {
                        text: "child_process".to_owned(),
                        kind: ModuleKind::Native,
                    }
                })
            },
        )
    };

    let module = compile(
        "import { execFile, execFileSync, exec, execSync, spawn } from 'child_process'; export { execFile, execFileSync, exec, execSync, spawn };",
    )
    .unwrap();
    assert!(module.native_dependencies().contains("child_process"));
    assert!(
        module
            .rust_source()
            .contains("modules::load(realm, \"child_process\")")
    );

    // `fork` is the Node library's, which importing it links.
    let forked = compile("import { fork } from 'child_process'; fork('tool');").unwrap();
    assert!(
        forked
            .native_dependencies()
            .contains("node:internal/electron/utility_process")
    );
    let diagnostics =
        compile("import { ChildProcess } from 'child_process'; new ChildProcess();").unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("module 'child_process' has no export 'ChildProcess'")
    }));
}

#[test]
fn exhausted_generic_array_iterators_keep_node_length_getter_behavior() {
    let output = result(
        r#"
        const out=[];
        for(const method of ['keys','values','entries']) {
          let fail=false;
          const target={get length(){if(fail)throw 'length';return this.n;},n:0};
          const it=Array.prototype[method].call(target);
          out.push(it.next().done);fail=true;
          try{it.next();}catch(error){out.push(error);}
          fail=false;target.n=Infinity;out.push(it.next().done);
        }
        export const result=out.join('|');
    "#,
    );
    assert_eq!(output, "true|length|true|true|length|true|true|length|true");
}

#[test]
fn generic_array_iterators_normalize_receivers_lengths_and_getter_progress() {
    let output = result(
        r#"
        const out = [];
        for (const method of ['values', 'keys', 'entries']) {
          for (const receiver of [null, undefined]) {
            try { Array.prototype[method].call(receiver); out.push('accepted'); }
            catch (error) { out.push(error.name); }
          }
        }
        for (const length of [NaN, undefined, -1, 0, 1.1, 2.9]) {
          const it = Array.prototype.values.call({0:'a', 1:'b', length});
          for (let n=0; n<3; n++) {
            const step=it.next(); out.push(String(step.value)+':'+step.done);
          }
        }
        const throwing = Array.prototype.values.call({length:2, get 0(){throw 'first';}, 1:'second'});
        try { throwing.next(); } catch (error) { out.push(error); }
        out.push(throwing.next().value);
        Object.defineProperty(Number.prototype, 'length', {configurable:true, get(){out.push(typeof this);return 0;}});
        Array.prototype.values.call(7).next();
        delete Number.prototype.length;
        export const result = out.join('|');
    "#,
    );
    assert_eq!(
        output,
        "TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|undefined:true|undefined:true|undefined:true|undefined:true|undefined:true|undefined:true|undefined:true|undefined:true|undefined:true|undefined:true|undefined:true|undefined:true|a:false|undefined:true|undefined:true|a:false|b:false|undefined:true|first|second|object"
    );
}

#[test]
fn array_iterator_getters_reenter_and_lengths_remain_live() {
    let output = result(
        r#"
        const out=[];
        for (const method of ['values','entries']) {
          let it;
          const target={length:2, get 0(){return it.next().value;}, 1:'b'};
          it=Array.prototype[method].call(target);
          out.push(String(it.next().value),it.next().done);
        }
        let lengthIt, reenter=true;
        const target={0:'a',1:'b',get length(){
          if(reenter){reenter=false;out.push(lengthIt.next().value);}
          return 2;
        }};
        lengthIt=Array.prototype.values.call(target);
        out.push(lengthIt.next().value,lengthIt.next().value,lengthIt.next().done);
        let fail=true;
        const retry=Array.prototype.values.call({0:'a',get length(){
          if(fail){fail=false;throw 'length';} return 1;
        }});
        try{retry.next();}catch(error){out.push(error);}
        out.push(retry.next().value,retry.next().done);
        const symbolic=Array.prototype.values.call({length:Symbol('length')});
        try{symbolic.next();}catch(error){out.push(error.name);}
        let reads=0;
        const growing={0:'a',get length(){reads++;return this.n;},n:1};
        const live=Array.prototype.keys.call(growing);
        out.push(live.next().value);
        growing.n=2;
        out.push(live.next().value,live.next().done);
        growing.n=3;
        out.push(live.next().done,reads);
        Object.defineProperty(growing,'0',{get(){throw 'keys read';}});
        out.push(Array.prototype.keys.call(growing).next().value);
        out.push([...Array.prototype.values.call('ab')].join(','));
        for(const value of [true,7,Symbol('x')]) out.push(Array.prototype.entries.call(value).next().done);
        export const result=out.join('|');
    "#,
    );
    assert_eq!(
        output,
        "b|true|0,1,b|true|a|a|b|true|length|a|true|TypeError|0|1|true|true|4|0|a,b|true|true|true"
    );
}

fn workspace_target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/task115b-native-tests")
}

/// Builds the start crate and the native value library once per test
/// process, as `nativeboat build` does: no Rust `std`, panics abort.
fn runtime() -> &'static PathBuf {
    static RUNTIME: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let built = RUNTIME.get_or_init(|| {
        std::panic::catch_unwind(build_runtime).map_err(|panic| {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
                .unwrap_or_else(|| "the build panicked".to_owned());
            // Every test needs the runtime, so a failed build fails the
            // others at once instead of each building it again, and its
            // error is shown while the tests still run.
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "the native runtime did not build: {message}"
            );
            message
        })
    });
    match built {
        Ok(directory) => directory,
        Err(message) => panic!("the native runtime did not build: {message}"),
    }
}

fn build_runtime() -> PathBuf {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    // The libraries are keyed by every source they compile and by the
    // compiler, reused by later test runs, and stale builds are removed,
    // so test runs neither rebuild them nor accumulate copies.
    let mut hasher = std::hash::DefaultHasher::new();
    let version = Command::new("rustc").arg("-vV").output().unwrap();
    std::hash::Hash::hash(&version.stdout, &mut hasher);
    for source in [
        ("nativeboat-native/src", true),
        ("nativeboat-start/src", false),
        ("nativeboat-regexp/src", false),
        ("nativeboat-regexp-build/src", false),
        ("nativeboat-regexp-build/overlays", false),
        ("nativeboat-regexp-build/vendor", false),
        ("nativeboat-url-build/src", false),
        ("nativeboat-url-build/overlays", false),
        ("nativeboat-url-build/vendor", false),
        ("nativeboat-encoding-build/src", false),
        ("nativeboat-cli/vendor/adler2-2.0.1", false),
        ("nativeboat-cli/vendor/miniz_oxide-0.9.1", false),
    ] {
        let root = crates.join(source.0);
        if source.1 {
            // The standalone rustc runtime build does not enable cfg(test).
            // Exclude the native crate's tests so coverage-only edits reuse
            // its production rlib instead of rebuilding the runtime.
            hash_tree_without_tests(&root, &mut hasher);
        } else {
            hash_tree(&root, &mut hasher);
        }
    }
    let name = format!("runtime-{:016x}", std::hash::Hasher::finish(&hasher));
    let directory = workspace_target().join(&name);
    let complete = |directory: &Path| {
        directory.join("libnativeboat_native.rlib").is_file()
            && directory.join("regexp/libnativeboat_regexp.rlib").is_file()
    };
    if complete(&directory) {
        return directory;
    }
    fs::create_dir_all(workspace_target()).unwrap();
    for entry in fs::read_dir(workspace_target()).unwrap().flatten() {
        let stale = entry.file_name().to_string_lossy().starts_with("runtime-");
        if stale && entry.file_name() != name.as_str() {
            // Another test process may still use it; then it stays.
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    let staging = workspace_target().join(format!(".{name}-{}", std::process::id()));
    fs::create_dir_all(&staging).unwrap();
    let cargo_dependencies = std::env::current_exe()
        .unwrap()
        .parent()
        .expect("test executable has a dependencies directory")
        .to_path_buf();
    let fingerprint_directory = cargo_dependencies
        .parent()
        .expect("test dependencies directory has a target directory")
        .join(".fingerprint");
    // Cargo's builds of miniz_oxide with the runtime's with-alloc
    // feature, non-SIMD first. A package's own tests build a separate
    // SIMD variant for another dev dependency; a workspace build unifies
    // the features into one build, whose dependencies are in the same
    // deps directory.
    let mut candidates: Vec<(bool, PathBuf)> = fs::read_dir(&fingerprint_directory)
        .unwrap()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let hash = name.strip_prefix("miniz_oxide-")?.to_owned();
            let fingerprint = fs::read_to_string(entry.path().join("lib-miniz_oxide.json")).ok()?;
            if !fingerprint.contains("\\\"with-alloc\\\"") {
                return None;
            }
            let simd = fingerprint.contains("\\\"simd\\\"");
            let rlib = cargo_dependencies.join(format!("libminiz_oxide-{hash}.rlib"));
            rlib.is_file().then_some((simd, rlib))
        })
        .collect();
    candidates.sort();
    let miniz_oxide = candidates
        .into_iter()
        .next()
        .map(|(_, rlib)| rlib)
        .expect("Cargo builds the native runtime's packaged miniz_oxide dependency");
    let matcher = staging.join("regexp");
    nativeboat_regexp_build::build(&matcher, &mut |args, directory| {
        let built = Command::new("rustc")
            .args(args)
            .current_dir(directory)
            .output()
            .map_err(|error| error.to_string())?;
        if built.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&built.stderr).into_owned())
        }
    })
    .unwrap();
    let url = staging.join("url");
    nativeboat_url_build::build(&url, &mut |args, directory| {
        let built = Command::new("rustc")
            .args(args)
            .current_dir(directory)
            .output()
            .map_err(|error| error.to_string())?;
        if built.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&built.stderr).into_owned())
        }
    })
    .unwrap();
    let encoding = staging.join("encoding");
    nativeboat_encoding_build::build_with_shared_memchr(
        &encoding,
        Some(&matcher.join("libmemchr.rlib")),
        &mut |args, directory| {
            let built = Command::new("rustc")
                .args(args)
                .current_dir(directory)
                .output()
                .map_err(|error| error.to_string())?;
            if built.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&built.stderr).into_owned())
            }
        },
    )
    .unwrap();
    let start = staging.join("start");
    fs::create_dir(&start).unwrap();
    fs::copy(
        crates.join("nativeboat-start/src/lib.rs"),
        start.join("lib.rs"),
    )
    .unwrap();
    let notices = format!(
        "{}\n{}\n{}\n{}",
        fs::read_to_string(crates.join("nativeboat-start/src/NOTICES.txt")).unwrap(),
        nativeboat_regexp_build::NOTICES,
        nativeboat_url_build::NOTICES,
        nativeboat_encoding_build::NOTICES
    );
    fs::write(start.join("NOTICES.txt"), notices).unwrap();
    let published_matcher = directory.join("regexp");
    for (name, library) in [
        ("nativeboat_start", "nativeboat-start/src/lib.rs"),
        ("nativeboat_native", "nativeboat-native/src/lib.rs"),
    ] {
        let source = if name == "nativeboat_start" {
            start.join("lib.rs")
        } else {
            crates.join(library)
        };
        let built = Command::new("rustc")
            .args(["--edition=2024", "--crate-type=rlib", "--crate-name", name])
            .args([
                "-C",
                "opt-level=1",
                "-C",
                "debuginfo=0",
                "-C",
                "panic=abort",
            ])
            .arg(format!(
                "--extern=nativeboat_regexp={}",
                matcher.join("libnativeboat_regexp.rlib").display()
            ))
            .arg(format!("-Ldependency={}", matcher.display()))
            .arg(format!(
                "--extern=url={}",
                url.join("liburl.rlib").display()
            ))
            .arg(format!(
                "--extern=idna={}",
                url.join("libidna.rlib").display()
            ))
            .arg(format!("-Ldependency={}", url.display()))
            .arg(format!(
                "--extern=chardetng={}",
                encoding.join("libchardetng.rlib").display()
            ))
            .arg(format!("-Ldependency={}", encoding.display()))
            .arg(format!("--extern=miniz_oxide={}", miniz_oxide.display()))
            .arg(format!("-Ldependency={}", cargo_dependencies.display()))
            .arg(source)
            .arg("--out-dir")
            .arg(&staging)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
    }
    // A concurrent test process may have published the same build. A
    // directory without the libraries, such as one a CI cache restored
    // without its files, is replaced.
    if fs::rename(&staging, &directory).is_err() {
        if !complete(&directory) {
            let _ = fs::remove_dir_all(&directory);
            let _ = fs::rename(&staging, &directory);
        }
        let _ = fs::remove_dir_all(&staging);
    }
    assert!(
        published_matcher
            .join("libnativeboat_regexp.rlib")
            .is_file()
    );
    directory
}

/// Hashes the names and contents of every file below `directory`.
fn hash_tree(directory: &Path, hasher: &mut std::hash::DefaultHasher) {
    let mut entries: Vec<_> = fs::read_dir(directory).unwrap().flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        std::hash::Hash::hash(&entry.file_name(), hasher);
        if path.is_dir() {
            hash_tree(&path, hasher);
        } else {
            std::hash::Hash::hash(&fs::read(&path).unwrap(), hasher);
        }
    }
}

fn hash_tree_without_tests(directory: &Path, hasher: &mut std::hash::DefaultHasher) {
    let mut entries: Vec<_> = fs::read_dir(directory).unwrap().flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if (path.is_dir() && name == "tests") || (!path.is_dir() && name == "tests.rs") {
            continue;
        }
        std::hash::Hash::hash(&name, hasher);
        if path.is_dir() {
            hash_tree_without_tests(&path, hasher);
        } else {
            std::hash::Hash::hash(&fs::read(&path).unwrap(), hasher);
        }
    }
}

/// Compiles a module, links it into a program whose `main` runs `main` (a
/// Rust block with `realm` and `exports` in scope) and returns its output.
fn execute(source: &str, main: &str) -> String {
    let module = compile_dynamic(source, Path::new("behavior.ts"))
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    run(&module, main)
}

/// Builds and runs a compiled program the way `nativeboat build` links one:
/// without Rust `std`, a C runtime, MSVC or Windows SDK libraries.
fn run(module: &DynamicModule, main: &str) -> String {
    run_with_entry(module, "generated::module(&mut realm)", main)
}

/// Builds and runs the generated native project with a custom root expression.
fn run_with_entry(module: &DynamicModule, entry: &str, main: &str) -> String {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = workspace_target().join(format!("{}-{id}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let program = directory.join("program.rs");
    let executable = directory.join(format!("program{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        &program,
        format!(
            "#![no_std]\n#![no_main]\nextern crate alloc;\nextern crate nativeboat_start;\n\
             macro_rules! println {{ ($($text:tt)*) => {{{{ ::nativeboat_native::platform::write_stdout(&::alloc::format!($($text)*)); ::nativeboat_native::platform::write_stdout(\"\\n\"); }}}} }}\n\
             mod generated {{\n{}\n}}\n\
             use nativeboat_native::{{ops, Key, Realm, Value}};\n\
             #[allow(unused_imports)] use alloc::{{borrow::ToOwned, format, string::String, vec, vec::Vec}};\n\
             fn show(realm: &mut Realm, value: &Value) -> String {{ ops::to_string(realm, value).map(|text| text.to_rust_lossy()).unwrap_or_else(|_| \"<unprintable>\".to_owned()) }}\n\
             #[unsafe(no_mangle)]\npub fn nativeboat_main() -> i32 {{\n\
             let mut realm = Realm::new();\n\
             let exports = match {entry} {{ Ok(exports) => Value::Object(exports), Err(thrown) => {{ let text = show(&mut realm, &thrown.0); println!(\"module threw {{text}}\"); return 0; }} }};\n\
             if let Err(thrown) = realm.run_event_loop() {{ let text = show(&mut realm, &thrown.0); println!(\"event loop threw {{text}}\"); }}\n\
             {main}\n0\n}}\n",
            module.rust_source(),
            entry = entry
        ),
    )
    .unwrap();
    let libraries = runtime();
    let compiled = Command::new("rustc")
        .args([
            "--edition=2024",
            "-C",
            "opt-level=0",
            "-C",
            "debuginfo=0",
            "-C",
            "panic=abort",
            "-D",
            "warnings",
        ])
        .args(["-C", "linker=rust-lld", "-C", "linker-flavor=lld-link"])
        .args([
            "-C",
            "link-arg=/NODEFAULTLIB",
            "-C",
            "link-arg=/ENTRY:nativeboat_start",
        ])
        .args(["-C", "link-arg=/SUBSYSTEM:CONSOLE"])
        // Deep native recursion gets the same reservation as built applications.
        .args(["-C", "link-arg=/STACK:268435456"])
        .env_remove("LIB")
        .env_remove("LIBPATH")
        .arg("-L")
        .arg(libraries)
        .arg(format!(
            "-Ldependency={}",
            libraries.join("regexp").display()
        ))
        .arg(format!("-Ldependency={}", libraries.join("url").display()))
        .arg(format!(
            "-Ldependency={}",
            libraries.join("encoding").display()
        ))
        .arg(format!(
            "-Ldependency={}",
            std::env::current_exe()
                .unwrap()
                .parent()
                .expect("test executable has a dependencies directory")
                .display()
        ))
        .arg("--extern")
        .arg(format!(
            "nativeboat_native={}",
            libraries.join("libnativeboat_native.rlib").display()
        ))
        .arg("--extern")
        .arg(format!(
            "nativeboat_start={}",
            libraries.join("libnativeboat_start.rlib").display()
        ))
        .arg(&program)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new(&executable).output().unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    remove_executed_fixture(&directory);
    String::from_utf8_lossy(&executed.stdout).trim().to_owned()
}

/// Windows can briefly retain a mapped executable after its process exits.
/// Retry only access/sharing/lock errors, with a finite one-second budget.
fn remove_executed_fixture(directory: &Path) {
    for attempt in 0..20 {
        match fs::remove_dir_all(directory) {
            Ok(()) => return,
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32 | 33)) && attempt < 19 => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(error) => panic!(
                "cannot remove executed fixture {}: {error}",
                directory.display()
            ),
        }
    }
}

/// Runs a module and prints its `result` export.
fn result(source: &str) -> String {
    execute(
        source,
        "match ops::get(&mut realm, &exports, &Key::name(\"result\")) {\n\
         Ok(value) => { let text = show(&mut realm, &value); println!(\"{text}\"); }\n\
         Err(thrown) => { let text = show(&mut realm, &thrown.0); println!(\"export threw {text}\"); }\n\
         }",
    )
}

/// Whether replacing a native global compiles to the TypeError it throws
/// when it runs: compiled code calls the global's members directly.
fn replaces_global(source: &str, name: &str) -> bool {
    compile_dynamic(source, Path::new("blocked.ts"))
        .unwrap()
        .rust_source()
        .contains(&format!(
            "Replacing the global {name} is not supported natively"
        ))
}

fn errors(source: &str) -> Vec<String> {
    compile_dynamic(source, Path::new("blocked.ts"))
        .unwrap_err()
        .into_iter()
        .map(|error| error.to_string())
        .collect()
}

#[test]
fn values_operators_and_conversions() {
    let output = result(
        r#"
        const parts = [];
        parts.push(1 + 2 * 3 + "x" + (0.1 + 0.2));
        parts.push(`t${1 + 1}-${null}-${"a😀"}`);
        parts.push(typeof 1, typeof "s", typeof null, typeof undefined, typeof function () {}, typeof {});
        parts.push(1 / 0, -(1 / 0), NaN === NaN, 0 === -0, "10" < "9", 10 < 9, 2 <= 2, 3 >= 4, 5 > 4);
        parts.push(1 == "1", null == undefined, 1 != 2, 1 !== "1", !0, ~5, -"3", +"0x10", void 0);
        parts.push(7 % 3, 2 ** 10, 5 & 3, 5 | 3, 5 ^ 3, 1 << 4, -16 >> 2, -16 >>> 28, 9 - 4, 8 / 2);
        parts.push("k" in { k: 1 }, "length" in [], 1e21, 1e-7, 5e-324, Infinity, delete 0);;
        parts.push(0 || "or", 1 && "and", null ?? "nullish", 0 ?? "zero", (1, 2), true ? "yes" : "no");
        export const result = parts.join(",");
        "#,
    );
    assert_eq!(
        output,
        "7x0.30000000000000004,t2-null-a😀,number,string,object,undefined,function,object,\
         Infinity,-Infinity,false,true,true,false,true,false,true,\
         true,true,true,true,true,-6,-3,16,,\
         1,1024,1,7,6,16,-4,15,5,4,\
         true,true,1e+21,1e-7,5e-324,Infinity,true,\
         or,and,nullish,0,2,yes"
    );
}

#[test]
fn bindings_closures_assignment_and_updates() {
    let output = result(
        r#"
        function counter(start = 10) {
          let count = start;
          return {
            next() { count += 1; return count; },
            get value() { return count; },
            set value(next) { count = next; },
          };
        }
        const made = counter(), other = counter(0);
        made.next();
        made.value = made.value * 2;
        let total = 0, text = "a";
        total += 5; total -= 1; total *= 3; total /= 4; total %= 2; total **= 3;
        total <<= 2; total >>= 1; total >>>= 0; total |= 8; total &= 12; total ^= 1;
        text += "b";
        let maybe = null, kept = "kept", empty = "";
        maybe ??= "filled"; kept ||= "lost"; empty ||= "filled"; kept &&= "replaced";
        const box = { n: 1, list: [1, 2] };
        box.n++; ++box.n; box["n"] += 1; box.list[1]--; box.extra = "e";
        let i = 5; const post = i++, pre = ++i; i--; --i;
        var hoisted = typeof later; var later = 1;
        var bare; var shared = "s"; const readShared = () => shared; shared += "!";
        function adder(x) { return (y) => x + y; }
        function nothing() { function helper() { return 0; } return; }
        { function blockScoped() { return "b"; } text += blockScoped(); }
        const fns = [];
        try { throw "caught"; } catch (error) { fns.push(() => error); }
        const methods = { name: "m", call() { return this.name; } };
        const viaKey = methods["call"](), optionalKey = methods?.["call"]();
        export const result = [made.value, other.next(), total, text, maybe, kept, empty,
          box.n, box.list.join("+"), box.extra, post, pre, i, hoisted, counter.length,
          bare, readShared(), adder(2)(3), nothing(), fns.pop()(), viaKey, optionalKey].join(" ");
        "#,
    );
    assert_eq!(
        output,
        "22 1 9 abb filled replaced filled 4 1+1 e 5 7 5 undefined 0  s! 5  caught m m"
    );
}

#[test]
fn control_flow_loops_and_labels() {
    let output = result(
        r#"
        const out = [];
        let n = 0;
        while (n < 10) { n++; if (n % 2) continue; if (n > 6) break; out.push(n); }
        do { n--; } while (n > 5);
        out.push(n);
        let sum = 0;
        for (let i = 0, j = 10; i < j; i++, j--) sum += i * j;
        out.push(sum);
        outer: for (let a = 0; a < 3; a++) {
          for (let b = 0; b < 3; b++) {
            if (b === 1) continue outer;
            if (a === 2) break outer;
            out.push(a + "" + b);
          }
        }
        let k = 0;
        for (k = 1; ; k *= 2) { if (k > 40) break; }
        for (;;) { k++; if (k > 70) break; }
        out.push(k);
        if (out.length > 100) out.push("never"); else if (out.length) { out.push("chained"); };
        first: second: for (let x = 0; x < 5; x++) { if (x === 1) break first; out.push("x" + x); }
        export const result = out.join(" ");
        "#,
    );
    assert_eq!(output, "2 4 6 5 70 00 10 71 chained x0");
}

#[test]
fn exceptions_try_catch_finally() {
    let output = result(
        r#"
        const log = [];
        function risky(kind) {
          if (kind === 1) throw "plain";
          if (kind === 2) return null.field;
          if (kind === 3) return undefined.call();
          return "fine";
        }
        for (let kind = 0; kind < 4; kind++) {
          try { log.push(risky(kind)); }
          catch (error) { log.push(typeof error === "string" ? error : error.message); }
          finally { log.push("f" + kind); }
        }
        try { try { throw "inner"; } finally { log.push("cleanup"); } } catch (e) { log.push("outer:" + e); }
        try { log.push("no error"); } catch { log.push("unreached"); }
        try { throw { code: 7 }; } catch {} finally {}
        function nested() {
          try { throw "a"; } catch (first) {
            try { throw first + "b"; } catch (second) { return second; }
          }
        }
        log.push(nested());
        function rethrow() { try { throw "x"; } catch (e) { throw e + "y"; } finally { log.push("ran"); } }
        try { rethrow(); } catch (e) { log.push(e); }
        try { (() => { throw new Thing(); })(); } catch (e) { log.push(e instanceof Thing); }
        function Thing() {}
        function deep(n) { return n === 0 ? 0 : 1 + deep(n - 1); }
        log.push(deep(500));
        function forever() { return forever(); }
        try { forever(); } catch (e) { log.push(e.name); }
        export const result = log.join("|");
        "#,
    );
    assert!(
        output.starts_with("fine|f0|plain|f1|Cannot read properties of null (reading 'field')|f2|")
    );
    assert!(output.contains("Cannot read properties of undefined (reading 'call')|f3|cleanup|outer:inner|no error|ab|ran|xy|true|500|RangeError"), "{output}");
}

#[test]
fn objects_prototypes_this_and_optional_chains() {
    let output = result(
        r#"
        function Point(x, y) { this.x = x; this.y = y; }
        Point.prototype.sum = function () { return this.x + this.y; };
        const p = new Point(3, 4);
        const key = "dyn";
        const literal = { [key + 1]: "computed", 2: "two", "quoted key": "q", method() { return this.tag; }, tag: "T",
          arrow: () => typeof this, nested: { deeper: { value: 9 } } };
        const holder = { v: 5, run() { const inner = () => () => this.v; return inner()(); } };
        const out = [p.sum(), p instanceof Point, literal.dyn1, literal[2], literal["quoted key"], literal.method(),
          literal.arrow(), holder.run(), literal.nested?.deeper?.value, literal.missing?.deeper.value,
          literal.missing?.(), literal.method?.(), literal?.["tag"], delete literal.tag, "tag" in literal,
          delete literal["2"], Object === undefined];
        export const result = out.join(",");
        function Object() {}
        "#,
    );
    assert_eq!(
        output,
        "7,true,computed,two,q,T,undefined,5,9,,,T,T,true,false,true,false"
    );
}

#[test]
fn temporal_dead_zones_and_constants() {
    let output = result(
        r#"
        const log = [];
        try { log.push(early); } catch (e) { log.push(e.name + ": " + e.message); }
        let early = 1;
        function readLate() { return late; }
        try { readLate(); } catch (e) { log.push(e.message); }
        let late = 2;
        log.push(readLate());
        const fixed = 1;
        try { fixed = 2; } catch (e) { log.push(e.message); }
        function bump() { shared++; }
        let shared = 0;
        bump(); bump();
        log.push(shared);
        let unset;
        log.push(typeof unset);
        export const result = log.join(" | ");
        "#,
    );
    assert_eq!(
        output,
        "ReferenceError: Cannot access 'early' before initialization | Cannot access 'late' before initialization | 2 | Assignment to constant variable. | 2 | undefined"
    );
}

#[test]
fn exports_are_live_bindings() {
    let output = execute(
        r#"
        export let count = 0;
        export function increment() { count++; return count; }
        const internal = "i";
        export { internal as alias };
        "#,
        "let read = |realm: &mut Realm| { let value = ops::get(realm, &exports, &Key::name(\"count\")).unwrap(); show(realm, &value) };\n\
         let before = read(&mut realm);\n\
         let increment = ops::get(&mut realm, &exports, &Key::name(\"increment\")).unwrap();\n\
         ops::call(&mut realm, &increment, Value::Undefined, &[]).unwrap();\n\
         let after = read(&mut realm);\n\
         let alias = ops::get(&mut realm, &exports, &Key::name(\"alias\")).unwrap();\n\
         let alias = show(&mut realm, &alias);\n\
         println!(\"{before} {after} {alias}\");",
    );
    assert_eq!(output, "0 1 i");
}

#[test]
fn module_errors_and_typescript_wrappers() {
    let output = result(
        r#"
        const value = (1 as number)! + (<number>2) + (3 satisfies number);
        let target: number = 0;
        (target as number) = value;
        (target!) += 1;
        (target satisfies number) *= 1;
        (<number>target) -= 0;
        ((target) as number) += 0;
        export const result = target;
        "#,
    );
    assert_eq!(output, "7");
    // Wrapped callees still call their methods on their objects.
    let receivers = result(
        r#"
        class Table { static create() { return this.name; } static make() { return this.create(); } }
        const holder: { make?: () => string } = Table;
        const tag = { name: "t", tag(strings: TemplateStringsArray) { return this.name + strings[0]; } };
        export const result = [
            holder.make!(), (Table.make)(), (Table.make as () => string)(),
            (Table.make satisfies () => string)(), (<() => string>Table.make)(), (tag.tag!)`x`,
        ].join(" ");
        "#,
    );
    assert_eq!(receivers, "Table Table Table Table Table tx");
    let typed = result(
        r#"
        import type { Shape } from "./shapes";
        export type Alias = string;
        export interface Named { name: string }
        type Local = number;
        interface Other { value: Local }
        declare global { interface Window { __NATIVEBOAT_TEST__?: string } }
        declare const ambient: number;
        declare function external(): void;
        function overloaded(value: string): string;
        function overloaded(value: number): number;
        function overloaded(value: unknown) { return value; }
        export const result = overloaded("types erased");
        "#,
    );
    assert_eq!(typed, "types erased");
    let thrown = result("throw new Failure(); function Failure() { this.message = 'top'; }");
    assert_eq!(thrown, "module threw [object Object]");
    let lone = result(
        r#"export const result = "\uD800".length + "�".length + `\uDC00`.length + "a\uD800b".length;"#,
    );
    assert_eq!(lone, "6");
}

#[test]
fn unsupported_constructs_are_all_reported_with_locations() {
    let reported = errors(
        "import x from 'y';\n\
         export * from 'z';\n\
         export { q } from 'z';\n\
         export default 1;\n\
         export enum K { A }\n\
         export const [z] = [1];\n\
         class C { accessor x = 1; }\n\
         async function* a() { yield* []; }\n\
         function* g() {}\n\
         const [d] = [1];\n\
         const s = [...d, , 2];\n\
         call(...s);\n\
         const o = { ...s, __proto__: null, async m() {}, async *n() {} };\n\
         switch (1) {}\n\
         for (const e of s) {}\n\
         label: { break label; }\n\
         function f() { try { return 1; } finally {} }\n\
         function h() { for (;;) { try { break; } finally {} } }\n\
         for (let i = 0; i < 1; i++) { () => i; }\n\
         const named = function self() { return self; };\n\
         Proxy.revocable({}, {});\n\
         undeclaredGlobal = 1;\n\
         /re/;\n\
         ({ d } = o);\n\
         function rest(...items) {}\n\
         function pattern({ p }) {}\n\
         try {} catch ([e]) {}\n\
         const arrowAsync = async () => 1;\n\
         `${tag}`;\n\
         tag`x`;\n\
         outer: if (1) {}\n\
         (o.d as any) = 1;\n\
         (o.d as any)++;\n\
         o?.d!;\n\
         for await (const e of s) {}\n\
         [(o.d as any)] = [];\n\
         debugger;\n\
         import.meta.resolve;\n\
         x;\n",
    );
    let expected = [
        "blocked.ts:1:15: cannot resolve module 'y'",
        "blocked.ts:2:15: cannot resolve module 'z'",
        "blocked.ts:7:11: auto-accessors are not compiled yet",
        "import.meta.resolve is not compiled yet",
    ];
    for message in expected {
        assert!(
            reported.iter().any(|error| error.contains(message)),
            "missing {message:?} in {reported:#?}"
        );
    }
    assert!(
        reported
            .iter()
            .all(|error| !error.contains("exported declaration is not compiled")),
        "TypeScript enums now export native bindings: {reported:#?}"
    );
    assert!(
        reported
            .iter()
            .all(|error| !error.contains("labels on non-loop statements")),
        "labeled blocks now lower to native control flow: {reported:#?}"
    );
}

#[test]
fn labeled_blocks_break_to_the_matching_statement_and_run_finally() {
    assert_eq!(
        execute(
            "export function verify() { let result = ''; outer: { result += 'a'; inner: if (true) { result += 'b'; break outer; } result += 'x'; } exit: { try { result += 'c'; break exit; } finally { result += 'd'; } result += 'x'; } return result; }",
            "let function = ops::get(&mut realm, &exports, &Key::name(\"verify\")).unwrap(); let value = ops::call(&mut realm, &function, Value::Undefined, &[]).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "abcd"
    );
    assert_eq!(
        execute(
            "export function verify(iteratorKey) { let closed = 0; const values = {[iteratorKey]() { return {next() { return {done:false, value:1}; }, return() { closed++; return {done:true}; }}; }}; leave: { for (const value of values) { break leave; } } return closed; }",
            "let function = ops::get(&mut realm, &exports, &Key::name(\"verify\")).unwrap(); let key = Value::Symbol(realm.symbols.iterator.clone()); let value = ops::call(&mut realm, &function, Value::Undefined, &[key]).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "1"
    );
}

#[test]
fn unbound_global_reads_preserve_reference_and_typeof_semantics() {
    let module = compile_dynamic(
        r#"
        globalThis.answer = 42;
        let missing;
        try { missing = notDefined; } catch (error) { missing = error.name + ":" + error.message; }
        export const result = [answer, typeof notDefined,
          typeof globalThis === "object" && globalThis.Object === Object, missing].join("|");
        "#,
        Path::new("globals.ts"),
    )
    .unwrap();
    let rust = module.rust_source();
    assert!(rust.contains("ops::get_global(realm, \"answer\")"));
    assert!(rust.contains("ops::get_global(realm, \"notDefined\")"));
    assert!(rust.contains("ops::type_of_global(realm, \"notDefined\")"));
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "42|undefined|true|ReferenceError:notDefined is not defined"
    );
}

#[test]
fn ambient_global_assignments_match_node_store_order_and_strict_errors() {
    let module = compile_dynamic(
        include_str!("tests/global_assignment.mjs"),
        Path::new("globals.mjs"),
    )
    .unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "native|ReferenceError|1|7|ReferenceError|undefined|1|3|7|7|8|8|11|12|13|TypeError|3"
    );
    let wrapped = compile_dynamic("globalThis.wrappedAmbient = 1; (wrappedAmbient as number) = 2; export const result = wrappedAmbient;", Path::new("globals.ts")).unwrap();
    assert_eq!(
        run(
            &wrapped,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "2"
    );
}

#[test]
fn sloppy_commonjs_global_stores_create_properties_and_ignore_refused_writes() {
    let module = mixed("entry.cjs", &[("entry.cjs", "ambientSloppy = 1; Object.defineProperty(globalThis, 'fixedSloppy', {value: 2}); fixedSloppy = 8; undefined = 10; function strict() { 'use strict'; try { strictMissing = 1; } catch(error) { return error.name; } } module.exports = [ambientSloppy, fixedSloppy, typeof undefined, strict()].join('|');")]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "1|2|undefined|ReferenceError"
    );
}

/// Programs that replace a native global only when it is missing compile,
/// as Emscripten's glue replaces `console`; a replacement that runs throws.
#[test]
fn native_global_replacements_compile_and_throw_when_run() {
    let module = mixed("entry.cjs", &[("entry.cjs", "'undefined' == typeof console && (console = {}); let caught; try { JSON = 3; } catch (error) { caught = error.name + ': ' + error.message; } module.exports = [typeof JSON.stringify, caught].join('|');")]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "function|TypeError: Replacing the global JSON is not supported natively"
    );
}

#[test]
fn syntax_errors_and_unknown_paths_fail_before_generation() {
    let syntax = compile_dynamic("let = ;", Path::new("broken.js")).unwrap_err();
    assert!(syntax[0].to_string().starts_with("broken.js:1:"));
    let semantic = compile_dynamic("let a; let a;", Path::new("twice.js")).unwrap_err();
    assert!(semantic[0].to_string().contains("twice.js"));
    let path = compile_dynamic("", Path::new("data.json")).unwrap_err();
    assert!(
        path[0].to_string().starts_with("data.json:1:1: "),
        "{path:?}"
    );
    let module = compile_dynamic("export const value = 1;", Path::new("ok.mjs")).unwrap();
    assert!(
        module
            .rust_source()
            .contains("pub fn module(realm: &mut Realm)")
    );
}

fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(name.as_str(), "node_modules" | "__tests__" | "dist" | "out") {
                sources(&path, found);
            }
        } else if (name.ends_with(".ts") || name.ends_with(".js")) && !name.ends_with(".d.ts") {
            found.push(path);
        }
    }
}

/// Compiles every desktop and Muya source module of unchanged MarkText and
/// ranks what blocks native compilation. Vue single-file components are
/// counted separately; their scripts need component compilation first.
#[test]
#[ignore = "requires NATIVEBOAT_MARKTEXT_PROJECT with unchanged MarkText sources"]
fn unchanged_marktext_blocker_inventory() {
    let project = PathBuf::from(
        std::env::var_os("NATIVEBOAT_MARKTEXT_PROJECT").expect("MarkText source root"),
    );
    let mut files = Vec::new();
    for package in ["packages/desktop/src", "packages/muya/src"] {
        sources(&project.join(package), &mut files);
    }
    files.sort();
    let mut compiled = 0;
    let mut blockers: std::collections::BTreeMap<String, usize> = Default::default();
    let mut blocked_files: std::collections::BTreeMap<String, usize> = Default::default();
    for file in &files {
        let source = fs::read_to_string(file).unwrap();
        match compile_dynamic(&source, file) {
            Ok(_) => compiled += 1,
            Err(errors) => {
                let mut kinds = std::collections::BTreeSet::new();
                for error in errors {
                    let message = if error.message.starts_with("global '") {
                        "global (built-in or ambient) is not compiled yet".to_owned()
                    } else {
                        error.message
                    };
                    *blockers.entry(message.clone()).or_default() += 1;
                    kinds.insert(message);
                }
                for kind in kinds {
                    *blocked_files.entry(kind).or_default() += 1;
                }
            }
        }
    }
    let mut ranked: Vec<_> = blockers.into_iter().collect();
    ranked.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let report = ranked
        .iter()
        .map(|(message, count)| {
            format!(
                "{count:6} sites in {:4} files: {message}",
                blocked_files[message]
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let summary = format!(
        "{} source modules, {compiled} compile natively now\n{report}\n",
        files.len()
    );
    fs::create_dir_all(workspace_target()).unwrap();
    fs::write(workspace_target().join("marktext-blockers.txt"), &summary).unwrap();
    println!("{summary}");
    assert!(!files.is_empty());
}

/// Compiles in-memory modules; specifiers name files directly.
fn link(files: &[(&str, &str)]) -> Result<DynamicModule, Vec<String>> {
    let files: std::collections::HashMap<PathBuf, String> = files
        .iter()
        .map(|(name, text)| (PathBuf::from(name), (*text).to_owned()))
        .collect();
    let known = files.clone();
    compile_project(
        Path::new("main.ts"),
        &mut |_, specifier| {
            let path = PathBuf::from(specifier);
            (known.contains_key(&path) || specifier.starts_with("missing")).then_some(path)
        },
        &mut |path| files.get(path).cloned(),
    )
    .map_err(|errors| errors.into_iter().map(|error| error.to_string()).collect())
}

/// Mixed source graph: only .cjs entries select CommonJS semantics.
fn mixed(entry: &str, files: &[(&str, &str)]) -> Result<DynamicModule, Vec<crate::Diagnostic>> {
    let files: std::collections::HashMap<PathBuf, ModuleSource> = files
        .iter()
        .map(|(name, text)| {
            (
                PathBuf::from(name),
                ModuleSource {
                    text: (*text).to_owned(),
                    kind: if name.ends_with(".json") {
                        ModuleKind::Json
                    } else if name.ends_with(".native") {
                        ModuleKind::Native
                    } else if name.ends_with(".cjs") || name.ends_with(".unknown") {
                        ModuleKind::CommonJs
                    } else {
                        ModuleKind::EsModule
                    },
                },
            )
        })
        .collect();
    compile_sources(
        Path::new(entry),
        &mut |_, name, _| {
            (!name.starts_with("unresolved"))
                .then(|| PathBuf::from(name.strip_prefix("./").unwrap_or(name)))
        },
        &mut |path| files.get(path).cloned(),
    )
}

/// Mixed source graph with several independently evaluated application roots.
fn mixed_entries(
    entries: &[&str],
    files: &[(&str, &str)],
) -> Result<DynamicModule, Vec<crate::Diagnostic>> {
    let files: std::collections::HashMap<PathBuf, ModuleSource> = files
        .iter()
        .map(|(name, text)| {
            (
                PathBuf::from(name),
                ModuleSource {
                    text: (*text).to_owned(),
                    kind: if name.ends_with(".json") {
                        ModuleKind::Json
                    } else if name.ends_with(".native") {
                        ModuleKind::Native
                    } else if name.ends_with(".cjs") || name.ends_with(".unknown") {
                        ModuleKind::CommonJs
                    } else {
                        ModuleKind::EsModule
                    },
                },
            )
        })
        .collect();
    let entries: Vec<_> = entries.iter().map(Path::new).collect();
    compile_sources_with_entries(
        &entries,
        &mut |_, name, _| (!name.starts_with("unresolved")).then(|| name.into()),
        &mut |path| files.get(path).cloned(),
    )
}

#[test]
fn commonjs_factories_are_lazy_cached_and_preserve_exports_aliases() {
    let module = mixed("main.ts", &[
        ("main.ts", "import value from 'dep.cjs'; import same from 'dep.cjs'; import alias from 'alias.cjs'; export const result = [value === same, value.result, value.thisMatches, value.lazy(), value.lazy(), alias.value, alias.discarded].join('|');"),
        ("dep.cjs", "'use strict'; exports.thisMatches = this === exports; let runs = 0; exports.result = require('count.cjs').next(); exports.lazy = () => require('count.cjs').next(); if (false) require('never.cjs'); return; throw 'past return';"),
        ("count.cjs", "'use strict'; let runs = 0; module.exports = {next: () => ++runs};"),
        ("never.cjs", "'use strict'; throw 'not lazy';"),
        ("alias.cjs", "'use strict'; exports = {discarded: 1}; module.exports = {value: 7}; exports.discarded = 2;"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "true|1|true|2|3|7|"
    );
}

#[test]
fn typescript_compiled_to_commonjs_requires_and_reads_esmodule_defaults() {
    // A plugin's TypeScript that its own build compiles to CommonJS: it
    // calls `require`, and its default import of a CommonJS module that
    // sets `__esModule` is that module's `default`, as `tsc` emits it.
    // TypeScript elsewhere keeps Node's interop: the default is
    // `module.exports`.
    let files: std::collections::HashMap<PathBuf, ModuleSource> = [
        ("plugin/main.ts", "import Reporter from 'reporter.cjs'; import plain from 'plain.cjs'; import * as other from 'other.ts'; const dep = require('dep.cjs'); export const result = [new Reporter().name, plain.kind, dep.value, other.kind].join('|');", ModuleKind::EsModule),
        ("reporter.cjs", "Object.defineProperty(exports, '__esModule', { value: true }); class Reporter { constructor() { this.name = 'reporter'; } } exports.default = Reporter;", ModuleKind::CommonJs),
        ("plain.cjs", "module.exports = { kind: 'plain' };", ModuleKind::CommonJs),
        ("dep.cjs", "exports.value = 'required';", ModuleKind::CommonJs),
        ("other.ts", "import Reporter from 'reporter.cjs'; export const kind = typeof Reporter;", ModuleKind::EsModule),
    ]
    .into_iter()
    .map(|(name, text, kind)| (PathBuf::from(name), ModuleSource { text: text.to_owned(), kind }))
    .collect();
    let options = CompileOptions {
        commonjs_folders: vec![PathBuf::from("plugin")],
        ..CompileOptions::default()
    };
    let module = compile_sources_with_options(
        Path::new("plugin/main.ts"),
        &mut |_, name, _| Some(PathBuf::from(name)),
        &mut |path| files.get(path).cloned(),
        &options,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "reporter|plain|required|object"
    );
}

#[test]
fn umd_factories_load_modules_through_their_require_parameter() {
    // jsonc-parser's UMD build passes the file's `require` to its factory,
    // which loads its parts through the parameter.
    let module = mixed("main.ts", &[
        ("main.ts", "import parser from 'umd.cjs'; export const result = parser.format();"),
        ("umd.cjs", "(function (factory) { if (typeof module === 'object' && typeof module.exports === 'object') { var v = factory(require, exports); if (v !== undefined) module.exports = v; } else if (typeof define === 'function' && define.amd) { define(['require', 'exports', './impl/format.cjs'], factory); } })(function (require, exports) { 'use strict'; var format = require('./impl/format.cjs'); exports.format = function () { return format.name; }; });"),
        ("impl/format.cjs", "exports.name = 'formatted';"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "formatted"
    );
}

#[test]
fn long_operator_chains_compile_and_keep_their_order() {
    // Generated tables concatenate hundreds of values in one expression;
    // the chain compiles without nesting as deep as it is long.
    let terms: Vec<String> = (0..600).map(|i| format!("t({i})")).collect();
    let source = format!(
        "const order: number[] = []; function t(i: number) {{ order.push(i); return i % 3 === 0 ? 's' : i; }}\n\
         const text = {};\n\
         const sum = 1000 - {};\n\
         export const result = [text.length, text.slice(0, 12), sum, order.length, order[0], order[599]].join('|');",
        terms.join(" + "),
        (1..=40)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" - ")
    );
    let text: String = (0..600)
        .map(|i| {
            if i % 3 == 0 {
                "s".to_owned()
            } else {
                i.to_string()
            }
        })
        .collect();
    let sum = 1000 - (1..=40).sum::<i32>();
    assert_eq!(
        result(&source),
        format!("{}|{}|{sum}|600|0|599", text.len(), &text[..12])
    );
}

#[test]
fn plugin_sources_compile_typescript_requires_type_reexports_and_module_values() {
    // `import x = require()` binds a CommonJS module's exports or an ES
    // module's namespace; a name another module exports only as a type is
    // dropped even when re-exported; `debugger` does nothing; and a
    // CommonJS file may pass its `module` object on, as Node's has `id`,
    // `path`, `require`, `loaded`, `children` and `paths`.
    let module = mixed("main.ts", &[
        ("main.ts", "import dep = require('dep.cjs'); import esm = require('esm.ts'); import { Shape, value } from 'shapes.ts'; export { Shape }; debugger; export const result = [dep.name, dep.loaded, dep.same, dep.kinds, esm.answer, value].join('|');"),
        ("dep.cjs", "'use strict'; function load(mod, request) { return mod.require(request); } const self = module; module.exports = { name: load(module, 'name.cjs'), same: self.id === self.filename && typeof self.path, kinds: [Array.isArray(module.children), Array.isArray(module.paths)].join(), get loaded() { return self.loaded; } }; require('name.cjs');"),
        ("name.cjs", "module.exports = 'cjs';"),
        ("esm.ts", "export const answer = 42;"),
        ("shapes.ts", "export type Shape = { side: number }; export interface Other {} export const value = 'v';"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "cjs|true|string|true,true|42|v"
    );
    // An exported alias of a required module is still reported.
    let errors = mixed(
        "main.ts",
        &[
            ("main.ts", "export import dep = require('dep.cjs');"),
            ("dep.cjs", "module.exports = 1;"),
        ],
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message
                == "exported TypeScript import = require() is not compiled yet"),
        "{errors:?}"
    );
}

#[test]
fn dynamic_imports_resolve_to_lazy_project_entries() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "console.log('before'); import('lazy.ts').then((ns) => console.log('loaded', ns.answer)); console.log('after');",
            ),
            (
                "lazy.ts",
                "console.log('lazy module evaluated'); export const answer = 42;",
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        module.entry_count(),
        2,
        "lazy imports are registered entries"
    );

    assert_eq!(
        run(&module, "let _ = exports; let _ = Key::name(\"answer\");"),
        "before\nafter\nlazy module evaluated\nloaded 42"
    );
}

#[test]
fn dynamic_import_scanning_reports_unresolved_missing_and_nonliteral_requests() {
    // A specifier known only at run time names no compiled module: the
    // build warns, and the import rejects when it runs.
    let nonliteral = compile_dynamic(
        "export const load = (name) => import(name);",
        Path::new("main.mjs"),
    )
    .unwrap();
    assert!(nonliteral.warnings().iter().any(|warning| {
        warning
            .message
            .contains("names no module the build can find")
    }));

    for (source, load, expected) in [
        (
            "export const load = () => import('./missing.mjs');",
            false,
            "cannot resolve dynamic import module",
        ),
        (
            "export const load = () => import('./missing.mjs');",
            true,
            "cannot read dynamic import module",
        ),
    ] {
        let errors = compile_sources(
            Path::new("main.mjs"),
            &mut |_, name, _| (load || name != "./missing.mjs").then(|| PathBuf::from(name)),
            &mut |path| {
                (path == Path::new("main.mjs")).then(|| ModuleSource {
                    text: source.to_owned(),
                    kind: ModuleKind::EsModule,
                })
            },
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{errors:?}"
        );
    }
}

#[test]
fn dynamic_import_attributes_remain_a_diagnostic() {
    let errors = compile_dynamic(
        "export const load = () => import('./data.json', { with: { type: 'json' } });",
        Path::new("main.mjs"),
    )
    .unwrap_err();
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("dynamic import attributes are not compiled yet")
    }));
}

#[test]
fn empty_and_duplicate_project_entries_preserve_the_project_contract() {
    let empty = compile_sources_with_entries(&[], &mut |_, _, _| None, &mut |_| None).unwrap_err();
    assert!(
        empty
            .iter()
            .any(|error| error.message.contains("at least one entry module"))
    );

    let entry = Path::new("main.mjs");
    let module = compile_sources_with_entries(&[entry, entry], &mut |_, _, _| None, &mut |path| {
        (path == entry).then(|| ModuleSource {
            text: "export const value = 42;".to_owned(),
            kind: ModuleKind::EsModule,
        })
    })
    .unwrap();
    assert_eq!(module.entries, 2);
}

#[test]
fn repeated_dynamic_imports_share_one_lazy_module_record() {
    let module = mixed(
        "main.mjs",
        &[
            ("main.mjs", "export const load = () => Promise.all([import('./lazy.mjs'), import('./lazy.mjs')]);"),
            ("lazy.mjs", "globalThis.loads = (globalThis.loads || 0) + 1; export const value = 7;"),
        ],
    ).unwrap();
    let output = run(
        &module,
        "let f = ops::get(&mut realm, &exports, &Key::name(\"load\")).unwrap(); ops::call(&mut realm, &f, Value::Undefined, &[]).unwrap(); realm.run_event_loop().unwrap(); let loads = ops::get_global(&mut realm, \"loads\").unwrap(); println!(\"{}\", show(&mut realm, &loads));",
    );
    assert_eq!(output, "1");
}

#[test]
fn create_require_rejects_esm_targets_with_a_source_diagnostic() {
    let errors = mixed(
        "main.mjs",
        &[
            ("main.mjs", "import { createRequire } from 'module'; const require = createRequire(import.meta.url); require('./dependency.mjs');"),
            ("dependency.mjs", "export const value = 1;"),
        ],
    ).unwrap_err();
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("createRequire cannot synchronously load non-CommonJS module")),
        "{errors:?}"
    );
}

#[test]
fn page_entry_set_shares_modules_and_keeps_dynamic_imports_lazy() {
    let module = mixed_entries(
        &["first.ts", "second.ts"],
        &[
            (
                "first.ts",
                "import { next } from 'shared.ts'; console.log('first', next()); export const first = true;",
            ),
            (
                "second.ts",
                "import { next } from 'shared.ts'; console.log('second', next()); import('lazy.ts').then((ns) => console.log('loaded', ns.value));",
            ),
            (
                "shared.ts",
                "console.log('shared'); let count = 0; export function next() { return ++count; }",
            ),
            (
                "lazy.ts",
                "import { next } from 'shared.ts'; console.log('lazy module'); export const value = next();",
            ),
        ],
    )
    .unwrap();
    assert_eq!(module.entry_count(), 3, "dynamic roots remain registered");
    assert_eq!(
        run_with_entry(
            &module,
            "generated::module_entries(&mut realm, &[0, 1]).map(|mut modules| modules.remove(0))",
            "let _ = ops::get(&mut realm, &exports, &Key::name(\"first\"));",
        ),
        "shared\nfirst 1\nsecond 2\nlazy module\nloaded 3"
    );
}

#[test]
fn module_create_require_links_local_commonjs_without_eager_evaluation() {
    let module = mixed(
        "main.mjs",
        &[
            (
                "main.mjs",
                "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url); console.log('before'); const dependency = require('dependency.cjs'); console.log('after'); export const result = dependency.answer;",
            ),
            (
                "dependency.cjs",
                "console.log('dependency evaluated'); module.exports = { answer: 42 };",
            ),
        ],
    )
    .unwrap();

    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "before\ndependency evaluated\nafter\n42"
    );
}

#[test]
fn module_create_require_resolve_folds_marktext_ripgrep_package_name() {
    let entry = Path::new("main.mjs");
    let source = "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url); const arch = process.env.npm_config_arch || process.arch; const binaryName = process.platform === 'win32' ? 'rg.exe' : 'rg'; const platformPkg = `@vscode/ripgrep-${process.platform}-${arch}`; export const binary = require.resolve(`${platformPkg}/bin/${binaryName}`); export const direct = require.resolve('fixed-package/path'); const stable = 'already-known' || 'fallback'; export const stablePath = require.resolve(stable);";
    let expected = format!(
        "@vscode/ripgrep-{}-{}/bin/{}",
        if cfg!(windows) {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        },
        match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            "x86" => "ia32",
            other => other,
        },
        if cfg!(windows) { "rg.exe" } else { "rg" },
    );
    let mut resolved = Vec::new();
    let mut loaded = Vec::new();
    let module = compile_sources_with_entries(
        &[entry],
        &mut |_, request, kind| {
            resolved.push((request.to_owned(), kind));
            match request {
                "node:module" => Some(PathBuf::from("node:module")),
                "fixed-package/path" | "already-known" => {
                    Some(PathBuf::from("C:/project/node_modules/fixed-package/path"))
                }
                request if request == expected => {
                    Some(PathBuf::from("C:/project/node_modules/ripgrep/bin/rg"))
                }
                _ => None,
            }
        },
        &mut |path| {
            loaded.push(path.to_path_buf());
            Some(if path == Path::new("node:module") {
                ModuleSource {
                    text: "node:module".to_owned(),
                    kind: ModuleKind::Native,
                }
            } else {
                ModuleSource {
                    text: source.to_owned(),
                    kind: ModuleKind::EsModule,
                }
            })
        },
    )
    .unwrap();

    assert!(resolved.contains(&(expected.clone(), RequestKind::Resolve)));
    assert!(resolved.contains(&("fixed-package/path".to_owned(), RequestKind::Resolve)));
    assert!(resolved.contains(&("already-known".to_owned(), RequestKind::Resolve)));
    assert!(
        !resolved
            .iter()
            .any(|(_, kind)| *kind == RequestKind::Require)
    );
    assert!(
        module
            .rust_source()
            .contains("Value::str(\"C:/project/node_modules/ripgrep/bin/rg\")")
    );
    assert!(!loaded.iter().any(|path| {
        path == Path::new("C:/project/node_modules/ripgrep/bin/rg")
            || path.ends_with("fixed-package/path")
    }));
}

#[test]
fn module_create_require_keeps_dynamic_and_escaped_uses_as_diagnostics() {
    for (source, expected) in [
        (
            "import { createRequire } from 'module'; const require = createRequire(import.meta.url); require(...names);",
            "createRequire needs one static string module specifier",
        ),
        (
            "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url); require.resolve(name);",
            "createRequire.resolve needs one statically known string specifier",
        ),
        (
            "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url); require.resolve(true);",
            "createRequire.resolve needs one statically known string specifier",
        ),
        (
            "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url); require.resolve(later); const later = 'known';",
            "createRequire.resolve needs one statically known string specifier",
        ),
        (
            "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url); require?.resolve('./dependency.cjs');",
            "optional require.resolve calls are not compiled yet",
        ),
        (
            "import { createRequire } from 'module'; const require = createRequire(import.meta.url); export { require };",
            "createRequire bindings cannot escape statically compiled require calls",
        ),
    ] {
        let diagnostics = compile_dynamic(source, Path::new("create-require.mjs")).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.to_string().contains(expected)),
            "{source}: {diagnostics:?}"
        );
    }
}

#[test]
fn commonjs_require_main_and_module_filename_point_to_the_entry() {
    let module = mixed(
        "main.cjs",
        &[(
            "main.cjs",
            "exports.result = [require.main?.filename, module.filename].join('|');",
        )],
    )
    .unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "main.cjs|main.cjs"
    );
}

#[test]
fn commonjs_require_main_is_undefined_for_an_esm_entry() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import value from 'dep.cjs'; export const result = value;",
            ),
            (
                "dep.cjs",
                "module.exports = require.main?.filename ?? 'undefined';",
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "undefined"
    );
}

#[test]
fn commonjs_module_detection_and_var_rebinding_match_wrapper_bindings() {
    let module = mixed("main.ts", &[
        ("main.ts", "import dayjs from 'dayjs.cjs'; import lodash from 'lodash.cjs'; import encodings from 'iconv.cjs'; export const result = [dayjs, lodash, encodings.utf8].join('|');"),
        ("dayjs.cjs", "module.exports = typeof module === 'object' && module && !module.nodeType && module.exports === exports ? 'ok' : 'bad';"),
        ("lodash.cjs", "var freeExports = typeof exports === 'object' && exports && !exports.nodeType && exports; var freeModule = freeExports && typeof module == 'object' && module && !module.nodeType && module; var moduleExports = freeModule && freeModule.exports === freeExports; module.exports = moduleExports ? 'ok' : 'bad';"),
        ("iconv.cjs", "var modules = [{utf8: 'ok'}]; for (var i = 0; i < modules.length; i++) { var module = modules[i]; for (var enc in module) if (Object.prototype.hasOwnProperty.call(module, enc)) exports[enc] = module[enc]; }"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "ok|ok|ok"
    );
}

#[test]
fn commonjs_cycles_expose_partial_exports_and_cache_completed_replacements() {
    let module = mixed("main.ts", &[
        ("main.ts", "import a from 'a.cjs'; export const result = [a.result, a.partial, a.other()].join('|');"),
        ("a.cjs", "'use strict'; exports.partial = 'early'; const b = require('b.cjs'); module.exports = {result: 'late', partial: b.seen, other: () => require('b.cjs').value};"),
        ("b.cjs", "'use strict'; const a = require('a.cjs'); exports.seen = a.partial; exports.value = 8;"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "late|early|8"
    );
}

#[test]
fn commonjs_partial_exports_support_inherited_classes_during_a_cycle() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import Stream from 'stream.cjs'; export const result = [typeof Stream.Duplex, Stream.duplexPair()].join('|');",
            ),
            (
                "stream.cjs",
                "function Stream() {} module.exports = Stream; Stream.Duplex = require('duplex.cjs'); Stream.duplexPair = require('duplexpair.cjs');",
            ),
            (
                "duplex.cjs",
                "function Duplex() {} module.exports = Duplex;",
            ),
            (
                "duplexpair.cjs",
                "const { Duplex } = require('stream.cjs'); class DuplexSide extends Duplex {} module.exports = () => typeof Duplex;",
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "function|function"
    );
}

#[test]
fn commonjs_imports_keep_depth_first_source_order_with_es_modules() {
    let module = mixed("main.ts", &[
        ("main.ts", "import 'a.cjs'; import {result} from 'b.ts'; export {result};"),
        ("a.cjs", "'use strict'; require('state.cjs').log.push('a');"),
        ("b.ts", "import state from 'state.cjs'; state.log.push('b'); export const result = state.log.join(',');"),
        ("state.cjs", "'use strict'; module.exports = {log: []};"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "a,b"
    );
}

#[test]
fn imported_bindings_reexport_the_original_live_cells_and_namespaces() {
    let files = [
        (
            "main.ts",
            "import value, {next, count, ns as original} from 'bridge.ts'; import * as ns from 'bridge.ts'; next(); export const result = [value, count, ns.count, original.count].join('|');",
        ),
        (
            "bridge.ts",
            "import defaultValue, {count, next} from 'value.ts'; import * as ns from 'value.ts'; export {defaultValue as default, count, next, ns};",
        ),
        (
            "value.ts",
            "export default 9; export let count = 0; export function next() { return ++count; }",
        ),
    ];
    assert_eq!(project_result(&files), "9|1|1|1");
}

#[test]
fn error_constructors_execute_native_brands_causes_and_prototype_relationships() {
    assert_eq!(
        result(
            "const alias = Error; const error = alias('message', {cause: 7}); const type = new TypeError('bad'); const plain = {}; const created = Error.call(plain); let readonly = false; try {Error.prototype = plain;} catch(error) {readonly = error instanceof TypeError;} let internal = false; try {Math.PI = 0;} catch(error) {internal = error instanceof TypeError && Error.isError(error);} const aggregate = AggregateError([error,type], 'many', {cause: 9}); const suppressed = new SuppressedError(error,type,'two'); const names = []; for (const kind of [EvalError,RangeError,ReferenceError,SyntaxError,URIError]) {names.push(new kind('x').name);} const other = names.join(','); export const result = [error.toString(),error.cause,error instanceof Error,type instanceof Error,type instanceof TypeError,type.constructor === TypeError,Error.isError(error),Error.isError(Error.prototype),Error.isError({name:'Error'}),created !== plain,created.hasOwnProperty('message'),readonly,internal,aggregate.errors[0] === error,aggregate.errors.length,aggregate.cause,suppressed.error === error,suppressed.suppressed === type,other].join('|');"
        ),
        "Error: message|7|true|true|true|true|true|false|false|true|false|true|true|true|2|9|true|true|EvalError,RangeError,ReferenceError,SyntaxError,URIError"
    );
    for name in [
        "Error",
        "EvalError",
        "RangeError",
        "ReferenceError",
        "SyntaxError",
        "TypeError",
        "URIError",
        "AggregateError",
        "SuppressedError",
    ] {
        assert!(replaces_global(&format!("{name} = 1;"), name));
    }
    assert_eq!(
        result(
            "function shadow(Error) {return new Error().answer;} function Local() {this.answer=42;} export const result = shadow(Local);"
        ),
        "42"
    );
}

#[test]
fn error_constructor_callbacks_preserve_order_and_original_exceptions() {
    assert_eq!(
        result(
            "let log = ''; const message = {toString() {log += 'm'; return 'bad';}}; const options = {get cause() {log += 'c'; throw 'cause';}}; let caught; try {new Error(message,options);} catch(error) {caught=error;} let type = false; try {new AggregateError();} catch(error) {type=error instanceof TypeError;} const suppressed = SuppressedError(1,2,undefined,options); export const result = [log,caught,type,suppressed.hasOwnProperty('error'),suppressed.hasOwnProperty('suppressed'),suppressed.hasOwnProperty('cause'),suppressed.hasOwnProperty('message')].join('|');"
        ),
        "mc|cause|true|true|true|false|false"
    );
    assert_eq!(
        execute(
            "export function verify(key) {let log = ''; const errors = {[key]() {log += 'i'; return {next() {log += 'n'; throw 'step';}, return() {throw 'close';}};}}; const message = {toString() {log += 'm'; return 'bad';}}; const options = {get cause() {log += 'c'; return 7;}}; let caught; try {AggregateError(errors,message,options);} catch(error) {caught=error;} return [log,caught].join('|');}",
            "let function = ops::get(&mut realm, &exports, &Key::name(\"verify\")).unwrap(); let key = Value::Symbol(realm.symbols.iterator.clone()); let value = ops::call(&mut realm, &function, Value::Undefined, &[key]).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "mcin|step"
    );
}

#[test]
fn function_apply_bind_execute_receivers_arguments_metadata_and_construction() {
    assert_eq!(
        result(
            "function collect(a,b,c) {return [this.tag,a,b,c,arguments.length].join(',');} const bound = collect.bind({tag:'bound'},1); const nested = bound.bind({tag:'ignored'},2); const arrayLike = {0:3,2:5,length:3.8}; function Point(x,y) {this.x=x;this.y=y;} const BoundPoint = Point.bind({x:99},7); const point = new BoundPoint(8); BoundPoint.prototype = {}; const same = new BoundPoint(9); const BoundError = TypeError.bind(null,'native'); const error = new BoundError(); function arrowFactory() {return (()=>this.tag).bind({tag:'ignored'});} const arrow = arrowFactory.call({tag:'lexical'}); let invalid = false; try {new arrow();} catch(error) {invalid=error instanceof TypeError;} export const result = [bound.apply({tag:'ignored'},[2,3]),nested(3),collect.apply({tag:'array'},arrayLike),collect.apply({tag:'empty'},null),bound.name,bound.length,bound.hasOwnProperty('prototype'),nested.name,nested.length,point.x,point.y,point instanceof Point,point instanceof BoundPoint,same instanceof BoundPoint,error instanceof TypeError,error instanceof BoundError,error.message,arrow(),invalid].join('|');"
        ),
        "bound,1,2,3,3|bound,1,2,3,3|array,3,,5,3|empty,,,,0|bound collect|2|false|bound bound collect|1|7|8|true|true|true|true|true|native|lexical|true"
    );
    assert_eq!(
        project_result(&[
            (
                "main.ts",
                "import collect from 'values.ts'; const apply = collect.apply; const bind = collect.bind; export const result = [apply.call(collect,6,{0:7,length:1}),bind.call(collect,8,9)()].join('|');"
            ),
            (
                "values.ts",
                "export default function collect(value) {return this + value;}"
            ),
        ]),
        "13|17"
    );
    assert_eq!(
        result(
            "function collect(a,b) {return [a,b,arguments.length].join(',');} function forward() {return collect.apply(null,arguments);} function receiver() {return this === undefined;} const input = {value:3}; const bound = ((value)=>value.value).bind(null,input); input.value=4; export const result = [forward(6,7),receiver.apply(),receiver.bind()(),Math.max.apply(null,[3,9,4]),bound()].join('|');"
        ),
        "6,7,2|true|true|9|4"
    );
}

#[test]
fn function_apply_keeps_getter_order_original_throws_and_explicit_resource_errors() {
    assert_eq!(
        result(
            "let trace = ''; function collect(a,b) {trace += 'c'; return [a,b].join(',');} const values = {get length() {trace += 'l'; return {valueOf() {trace += 'v'; return 2;}};}, get 0() {trace += '0'; return 7;}, get 1() {trace += '1'; throw 'index';}}; let caught; try {collect.apply(null,values);} catch(error) {caught=error;} const first = trace; trace=''; const apply = collect.apply; let invalid = false; try {apply.call({},null,values);} catch(error) {invalid=error instanceof TypeError;} let primitive = false; try {collect.apply(null,'ab');} catch(error) {primitive=error instanceof TypeError;} let limit = false; try {collect.apply(null,{length:Infinity,get 0() {throw 'index';}});} catch(error) {limit=error instanceof RangeError;} function fail() {throw 'body';} let body; try {fail.bind(null)();} catch(error) {body=error;} export const result = [first,caught,trace,invalid,primitive,limit,body].join('|');"
        ),
        "lv01|index||true|true|true|body"
    );
}

#[test]
fn symbols_execute_native_identity_registry_metadata_and_intrinsic_instance_hooks() {
    assert_eq!(
        result(
            "const alias = Symbol; const a = alias('tag'); const b = Symbol('tag'); const none = Symbol(); const empty = Symbol(''); const shared = Symbol.for('shared'); const key = Symbol('key'); const data = {[key]:7}; const copy = {...data}; let construct = false; try {new Symbol('x');} catch(error) {construct=error instanceof TypeError;} let readonly = false; try {Symbol.iterator = a;} catch(error) {readonly=error instanceof TypeError;} function Point() {} const point = new Point(); const BoundPoint = Point.bind(null); const intrinsic = Point[Symbol.hasInstance]; let hookReadonly = false; try {Point[Symbol.hasInstance] = ()=>false;} catch(error) {hookReadonly=error instanceof TypeError;} export const result = [typeof a,a !== b,a.description,none.description === undefined,empty.description === '',a.toString(),a.valueOf() === a,shared === Symbol.for('shared'),Symbol.keyFor(shared),Symbol.keyFor(a) === undefined,Symbol.keyFor(Symbol.iterator) === undefined,Symbol.prototype.constructor === Symbol,data[key],copy[key],construct,readonly,point instanceof BoundPoint,Point[Symbol.hasInstance](point),intrinsic.call({},point),intrinsic.name,intrinsic.length,hookReadonly,Symbol.asyncDispose.description,Symbol.unscopables.description].join('|');"
        ),
        "symbol|true|tag|true|true|Symbol(tag)|true|true|shared|true|true|true|7|7|true|true|true|true|false|[Symbol.hasInstance]|1|true|Symbol.asyncDispose|Symbol.unscopables"
    );
    assert!(replaces_global("Symbol = ()=>0;", "Symbol"));
    assert_eq!(
        result("function shadow(Symbol) {return Symbol();} export const result = shadow(()=>42);"),
        "42"
    );
}

#[test]
fn classes_construct_inherit_and_initialize_fields() {
    assert_eq!(
        result(
            r#"
            class Animal {
                legs = 4;
                #secret = 'hidden';
                static count = 0;
                static #registry = [];
                constructor(name) { this.name = name; Animal.count++; Animal.#registry.push(name); }
                speak() { return this.name + ' makes a sound'; }
                get description() { return this.name + '/' + this.legs; }
                set nickname(value) { this.alias = value; }
                get nickname() { return this.alias; }
                reveal() { return this.#secret; }
                #hidden() { return 'h:' + this.name; }
                callHidden() { return this.#hidden(); }
                static registry() { return Animal.#registry.join(','); }
                static isAnimal(value) { return #secret in value; }
                ['computed' + 'Name']() { return 'computed'; }
            }
            class Dog extends Animal {
                tricks = [];
                constructor(name) { super(name); this.legs = 4; }
                speak() { return super.speak() + ' (woof)'; }
                get description() { return 'dog ' + super.description; }
                static create(name) { return new this(name); }
            }
            const Cat = class Named extends Animal {
                speak() { return 'meow from ' + Named.name; }
            };
            const dog = Dog.create('Rex');
            const cat = new Cat('Tom');
            dog.nickname = 'R';
            let called = 'no';
            try { Dog('x'); } catch (error) { called = error instanceof TypeError; }
            export const result = [
                dog.speak(), cat.speak(), dog.description, dog.nickname, dog.reveal(), dog.callHidden(),
                Animal.count, Animal.registry(), Animal.isAnimal(dog), Animal.isAnimal({}), dog.computedName(),
                dog instanceof Dog, dog instanceof Animal, Object.getPrototypeOf(Dog) === Animal,
                typeof Dog, Dog.name, Cat.name, Dog.length, called, Object.keys(dog).join(','),
                Object.getOwnPropertyDescriptor(Animal.prototype, 'speak').enumerable,
            ].join('|');
            "#
        ),
        "Rex makes a sound (woof)|meow from Named|dog Rex/4|R|hidden|h:Rex|2|Rex,Tom|true|false|computed|true|true|true|function|Dog|Named|1|true|legs,name,tricks,alias|false"
    );
}

#[test]
fn class_constructors_follow_derived_this_and_return_rules() {
    assert_eq!(
        result(
            r#"
            const outcomes = [];
            const attempt = (make) => { try { return make(); } catch (error) { outcomes.push(error.constructor.name + ':' + error.message); } };
            class Base { constructor(value) { this.value = value; this.target = new.target.name; } }
            class Early extends Base { constructor() { this.x = 1; super(1); } }
            class Twice extends Base { constructor() { super(1); super(2); } }
            class Missing extends Base { constructor() {} }
            class Primitive extends Base { constructor() { super(1); return 5; } }
            class Replaced extends Base { constructor() { return {replaced: true}; } }
            class Arrow extends Base { constructor() { const call = () => super(7); call(); this.after = this.value; } }
            class Defaulted extends Base {}
            class Plain { constructor() { return 3; } }
            class Swap { constructor() { return [1]; } }
            class Empty {}
            class FromNull extends null {}
            attempt(() => new Early());
            attempt(() => new Twice());
            attempt(() => new Missing());
            attempt(() => new Primitive());
            attempt(() => new FromNull());
            attempt(() => { class Bad extends 5 {} });
            attempt(() => { function Odd() {} Odd.prototype = 3; class Bad extends Odd {} });
            attempt(() => { class Early extends Early {} });
            const arrow = new Arrow();
            const defaulted = new Defaulted('d');
            export const result = [
                outcomes.join(','), new Replaced().replaced, arrow.after, arrow.target, defaulted.value, defaulted.target,
                new Plain() instanceof Plain, new Swap()[0] === 1, new Empty() instanceof Empty,
                Object.getPrototypeOf(FromNull.prototype) === null, Defaulted.length, Base.length,
            ].join('|');
            "#
        ),
        "ReferenceError:Must call super constructor in derived class before accessing 'this' or returning from derived constructor,ReferenceError:Super constructor may only be called once,ReferenceError:Must call super constructor in derived class before accessing 'this' or returning from derived constructor,TypeError:Derived constructors may only return object or undefined,TypeError:Super constructor is not a constructor,TypeError:Class extends value is not a constructor or null,TypeError:Class extends value does not have valid prototype property,ReferenceError:Cannot access 'Early' before initialization|true|7|Arrow|d|Defaulted|true|true|true|true|0|1"
    );
}

#[test]
fn class_fields_static_blocks_and_super_bind_their_class() {
    assert_eq!(
        result(
            r#"
            let counter = 0;
            const key = () => 'k' + ++counter;
            class Parent {
                static tag = 'parent';
                static describe() { return 'P:' + this.tag; }
                greet() { return 'hi'; }
                ['x']() { return 'px'; }
            }
            class Child extends Parent {
                [key()] = counter;
                [key()] = counter;
                self = this;
                arrow = () => this;
                greeting = super.greet();
                'quoted key' = 1;
                static tag = 'child';
                static inherited = super.describe();
                static computed = super['describe']();
                static #count = 0;
                static {
                    var local = 'block';
                    this.fromBlock = local + ':' + this.tag;
                    Child.#count++;
                    const arrow = () => this.tag + Child.#count;
                    this.fromArrow = arrow();
                }
                static describe() { return 'C>' + super.describe(); }
                x() { return super.x() + '!' + super['greet'](); }
                target() { return new.target; }
            }
            class Only { static { Only.ready = true; } #check() { return 1; } }
            const Renamed = class Inner {};
            const child = new Child();
            export const result = [
                child.k1, child.k2, child.self === child, child.arrow() === child, child.greeting,
                Child.inherited, Child.computed, Child.fromBlock, Child.fromArrow, Child.describe(),
                child.x(), child.target(), child['quoted key'], counter,
                Object.keys(child).join(','), Only.ready, Renamed.name,
            ].join('|');
            "#
        ),
        "2|2|true|true|hi|P:child|P:child|block:child|child1|C>P:child|px!hi||1|2|k1,k2,self,arrow,greeting,quoted key|true|Inner"
    );
}

#[test]
fn private_elements_are_class_scoped_and_checked() {
    assert_eq!(
        result(
            r#"
            const outcomes = [];
            const attempt = (run) => { try { run(); } catch (error) { outcomes.push(error.constructor.name + ':' + error.message); } };
            class Counter {
                #value = 0;
                static #instances = 0;
                get #doubled() { return this.#value * 2; }
                set #doubled(value) { this.#value = value / 2; }
                get #readOnly() { return 'r'; }
                set #writeOnly(value) { this.log = value; }
                static get #total() { return Counter.#instances; }
                #method() { return 'm'; }
                constructor() { Counter.#instances++; this.#value++; this.#value += 2; }
                bump() { this.#doubled = 10; return [this.#value, this.#doubled, this.#readOnly, this.#method()].join(','); }
                optional(other) { return other?.#value; }
                writeOnly() { this.#writeOnly = 'w'; return this.log; }
                static total() { return Counter.#total; }
                static check(object) { return #value in object; }
                tryRead(object) { return object.#value; }
                tryWrite(object) { object.#value = 1; }
                tryMethod() { this.#method = 1; }
                tryGetter() { this.#readOnly = 1; }
                trySetter() { return this.#writeOnly; }
                closure() { return () => this.#value; }
                nested() {
                    class Inner { #value = 'inner'; read(o) { return o.#value; } }
                    return new Inner().read(new Inner());
                }
            }
            const counter = new Counter();
            new Counter();
            attempt(() => counter.tryRead({}));
            attempt(() => counter.tryWrite({}));
            attempt(() => counter.tryMethod());
            attempt(() => counter.tryGetter());
            attempt(() => counter.trySetter());
            attempt(() => Counter.check(1));
            attempt(() => { class Twice { constructor(o) { return o; } } class Stamp extends Twice { #mark = 1; } const target = {}; new Stamp(target); new Stamp(target); });
            export const result = [
                counter.bump(), counter.optional(null), counter.optional(counter), counter.writeOnly(), Counter.total(),
                Counter.check(counter), Counter.check({}), counter.closure()(), counter.nested(), outcomes.join(','),
            ].join('|');
            "#
        ),
        "5,10,r,m||5|w|2|true|false|5|inner|TypeError:Cannot read private member #value from an object whose class did not declare it,TypeError:Cannot write private member #value from an object whose class did not declare it,TypeError:Private method is not writable,TypeError:Private accessor was defined without a setter,TypeError:Private accessor was defined without a getter,TypeError:Cannot use 'in' operator to search for a private member in a primitive,TypeError:Cannot initialize a private member twice on the same object"
    );
}

/// Sloppy CommonJS keeps Annex B's block functions: a function declared in
/// a block or switch case is also a `var` of its function, given its value
/// when the declaration runs, unless a lexical declaration of the name is in
/// the way. The module object compares by identity, as `require.main ===
/// module` checks.
#[test]
fn sloppy_commonjs_block_functions_follow_annex_b() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import legacy from 'legacy.cjs'; export const result = legacy.join('|');",
            ),
            (
                "legacy.cjs",
                r#"var out = [];
out.push(typeof inner);
if (true) { out.push(typeof inner()); function inner() { return 1; } }
out.push(typeof inner);
function wrap() {
  if (true) { function nested() { return 'n'; } }
  return nested();
}
out.push(wrap());
switch (1) { case 1: function inCase() { return 'c'; } }
out.push(inCase());
let lexical = 'kept';
{ function lexical() {} }
out.push(lexical);
out.push(require.main === module, require.main !== module);
module.exports = out;
"#,
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "undefined|number|function|n|c|kept|false|true"
    );
}

#[test]
fn class_declarations_export_and_ignore_type_only_members() {
    assert_eq!(
        project_result(&[
            (
                "main.ts",
                "import Shape, {Square, make} from 'shapes.ts'; import Anonymous from 'anonymous.ts'; \
                 export const result = [new Shape().kind, new Square(2).area(), make().kind, new Anonymous().kind, Anonymous.name].join('|');",
            ),
            (
                "shapes.ts",
                "export default class Shape { kind: string = 'shape'; declare hidden: number; [key: string]: unknown; } \
                 export abstract class Base { abstract area(): number; abstract size: number; method(): void; method() {} } \
                 export class Square extends Base { side: number; constructor(side: number) { super(); this.side = side; } area() { return this.side * this.side; } } \
                 export function make() { return new Shape(); } \
                 declare class Ambient {}",
            ),
            (
                "anonymous.ts",
                "export default class { kind = 'anonymous'; }"
            ),
        ]),
        "shape|4|shape|anonymous|default"
    );
}

#[test]
fn typescript_parameter_properties_initialize_base_and_derived_instances() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "class Base { constructor(public value: number) { this.seen = value; } } \
             class Child extends Base { after = ''; constructor(readonly label: string, value: number) { super(value); this.after = this.label + this.value; } } \
             const child = new Child('item', 7); export const result = [child.value, child.seen, child.label, child.after].join('|');",
        )]),
        "7|7|item|item7"
    );
}

/// The `result` export of one module compiled with `options`, run natively.
fn module_result_with(name: &str, source: &str, options: &CompileOptions) -> String {
    let source = source.to_owned();
    let module = compile_sources_with_options(
        Path::new(name),
        &mut |_, _, _| None,
        &mut |_| {
            Some(ModuleSource {
                text: source.clone(),
                kind: ModuleKind::EsModule,
            })
        },
        options,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    run(
        &module,
        "match ops::get(&mut realm, &exports, &Key::name(\"result\")) {\n\
         Ok(value) => { let text = show(&mut realm, &value); println!(\"{text}\"); }\n\
         Err(thrown) => { let text = show(&mut realm, &thrown.0); println!(\"export threw {text}\"); }\n\
         }",
    )
}

/// TypeScript's `useDefineForClassFields: false`: parameter properties are
/// assigned before field initializers, which may read them; fields without
/// initializers declare only types, so they keep what a base constructor
/// set; initializers assign, so setters run. JavaScript modules keep define
/// semantics.
#[test]
fn typescript_class_fields_assign_after_parameter_properties() {
    let options = CompileOptions {
        assign_class_fields: true,
        ..CompileOptions::default()
    };
    let source = "class Service { constructor(public value: number) {} } \
         class Base { constructor(readonly service: Service) {} } \
         class Reader extends Base { doubled = this.service.value * 2; label: string; constructor(service: Service) { super(service); } } \
         class Own { tripled = this.service.value * 3; constructor(private readonly service: Service) {} } \
         class Init { constructor() { this.setup(); } setup() {} } \
         class Kept extends Init { value: number; override setup() { this.value = 5; } } \
         class Setter { stored = 0; set amount(value: number) { this.stored = value * 10; } } \
         class Amount extends Setter { amount = 4; } \
         const reader = new Reader(new Service(3)); const amount = new Amount(); \
         export const result = [reader.doubled, 'label' in reader, new Own(new Service(2)).tripled, new Kept().value, amount.stored, Object.hasOwn(amount, 'amount')].join('|');";
    assert_eq!(
        module_result_with("main.ts", source, &options),
        "6|false|6|5|40|false"
    );
    // Define semantics: an initializer runs before parameter properties and
    // an uninitialized field resets what the base constructor set.
    let define = "class Init { constructor() { this.setup(); } setup() {} } \
         class Kept extends Init { value; setup() { this.value = 5; } } \
         class Setter { stored = 0; set amount(value) { this.stored = value * 10; } } \
         class Amount extends Setter { amount = 4; } const amount = new Amount(); \
         export const result = [new Kept().value, amount.stored, Object.hasOwn(amount, 'amount')].join('|');";
    assert_eq!(module_result_with("main.js", define, &options), "|0|true");
    assert_eq!(
        module_result_with("main.ts", define, &CompileOptions::default()),
        "|0|true"
    );
}

/// TypeScript's `experimentalDecorators` as `tsc` applies them: member
/// decorators last first with the prototype (or class), key and descriptor,
/// a returned descriptor replacing the member; parameter decorators before
/// their member's; instance members, then static ones, then constructor
/// parameters (with the class and `undefined`), then the class.
#[test]
fn typescript_legacy_decorators_apply_as_tsc_applies_them() {
    let options = CompileOptions {
        legacy_decorators: true,
        assign_class_fields: true,
        ..CompileOptions::default()
    };
    let source = "const calls: string[] = []; \
         function log(name: string) { return (target: any, key: string, descriptor?: PropertyDescriptor) => { calls.push(name + ':' + key + ':' + (target === Box ? 'class' : target === Box.prototype ? 'proto' : '?') + ':' + typeof descriptor); }; } \
         function param(name: string) { return (target: any, key: string | undefined, index: number) => { calls.push(name + ':' + String(key) + ':' + index + ':' + (target === Box ? 'class' : 'proto')); }; } \
         function twice(target: any, key: string, descriptor: PropertyDescriptor) { const inner = descriptor.value; return { ...descriptor, value(this: any, ...args: number[]) { return inner.apply(this, args) * 2; } }; } \
         function stored(target: any, key: string): any { return { value: 'from ' + key, writable: true, enumerable: false, configurable: true }; } \
         function sealed(constructor: Function) { calls.push('class:' + constructor.name); } \
         @sealed class Box { \
           @log('static') static make() { return 1; } \
           @log('field') name: string; \
           @stored label: string; \
           @log('outer') @log('inner') measure(@param('p0') a: number, @param('p1') b: number) { return a + b; } \
           @twice three() { return 3; } \
           constructor(@param('c0') first: number, @param('c1') second: number) {} \
         } \
         const box = new Box(1, 2); \
         export const result = [calls.join(','), box.three(), box.measure(1, 2), box.label, Object.hasOwn(box, 'name')].join('|');";
    assert_eq!(
        module_result_with("main.ts", source, &options),
        "field:name:proto:undefined,p1:measure:1:proto,p0:measure:0:proto,inner:measure:proto:object,outer:measure:proto:object,static:make:class:object,c1:undefined:1:class,c0:undefined:0:class,class:Box|6|3|from label|false"
    );
}

#[test]
fn captured_derived_constructor_parameter_property_unwraps_before_initializing() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "class Base {} class Child extends Base { constructor(public muya: number) { super(); this.read = () => muya; } } export const result = new Child(42).read();",
        )]),
        "42"
    );
}

#[test]
fn captured_base_constructor_parameter_property_unwraps_before_initializing() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "class Base { constructor(public muya: number) { this.read = () => muya; } } export const result = new Base(42).read();",
        )]),
        "42"
    );
}

#[test]
fn typescript_class_decorators_run_inside_out_and_can_mutate_the_class() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "const order: string[] = []; function outer(target: any) { order.push('outer'); target.prototype.outer = true; } \
             function inner(target: any) { order.push('inner'); target.prototype.inner = true; } \
             @outer @inner class Example { static ready = true; } \
             const value = new Example(); export const result = [order.join(','), value.outer, value.inner, Example.ready].join('|');",
        )]),
        "inner,outer|true|true|true"
    );
}

#[test]
fn typescript_class_decorator_replacements_update_captured_declarations() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "function replace(target: any) { return class Replacement {} } \
             @replace class Example { static original = true; } \
             function current() { return Example; } \
             export const result = [current() === Example, Example.name, Example.original].join('|');",
        )]),
        "true|Replacement|"
    );
}

#[test]
fn typescript_enums_link_string_and_numeric_members_natively() {
    assert_eq!(
        project_result(&[
            (
                "main.ts",
                "import { CopyType, PasteType } from 'clipboard.ts'; import { SelectionType } from 'selection.ts'; import { Numeric } from 'numeric.ts'; \
                 export const result = [CopyType.NORMAL, CopyType.COPY_AS_HTML, PasteType.PASTE_AS_PLAIN_TEXT, SelectionType.TABLE, Numeric.Up, Numeric.Down, Numeric[6]].join('|');",
            ),
            (
                "clipboard.ts",
                "export enum CopyType { NORMAL = 'normal', COPY_AS_HTML = 'html' } export enum PasteType { NORMAL = 'normal', PASTE_AS_PLAIN_TEXT = 'plain' }",
            ),
            (
                "selection.ts",
                "export enum SelectionType { TEXT = 'text', TABLE = 'table' }",
            ),
            ("numeric.ts", "export enum Numeric { Up, Down = 5, Left }",),
        ]),
        "normal|html|plain|table|0|5|Left"
    );
}

#[test]
fn unsupported_class_features_are_reported() {
    let reported = errors(
        "@dec class D { @dec m() {} @dec f = 1; }\n\
         class P { constructor(private x: number) {} *gen() {} async *run() {} }\n\
         function plain() { return new.target; }\n\
         const literal = { m() { return super.x; } };\n",
    );
    for message in [
        "blocked.ts:1:16: decorators are not compiled yet",
        "blocked.ts:1:28: decorators are not compiled yet",
        "blocked.ts:4:32: super outside class methods is not compiled yet",
    ] {
        assert!(
            reported.iter().any(|error| error.contains(message)),
            "missing {message:?} in {reported:#?}"
        );
    }
    assert!(
        !reported
            .iter()
            .any(|error| error.contains("TypeScript parameter properties")),
        "parameter properties should compile: {reported:#?}"
    );
}

#[test]
fn arrays_execute_natively_with_species_iteration_and_copying_methods() {
    assert_eq!(
        result(
            r#"
            const values = [3, 1, 2];
            const doubled = values.map((x) => x * 2);
            const sorted = [...values].sort((a, b) => a - b);
            const evens = Array.from({ length: 5 }, (_, i) => i).filter((x) => x % 2 === 0);
            const flat = [1, [2, [3, [4]]]].flat(Infinity);
            const found = values.find((x) => x > 1);
            const reduced = values.reduce((a, b) => a + b, 10);
            const spliced = [0, 1, 2, 3];
            const removed = spliced.splice(1, 2, 'a');
            class Stack extends Array {}
            const stack = Stack.of(1, 2);
            const mappedStack = stack.map((x) => x);
            const cyclic = [1];
            cyclic.push(cyclic);
            export const result = [
                doubled.join(), sorted.join(), evens.join(), flat.join(), found, reduced,
                spliced.join(), removed.join(), Array.isArray(stack), stack instanceof Stack,
                mappedStack instanceof Stack, [1, 2, 3].includes(2), [1, 2, 3].at(-1),
                [..."ab"].join("|"), Array(3).length, new Array(1, 2).toReversed().join(),
                [5, 1, 4].toSorted().join(), [1, 2, 3].with(1, 9).join(), '' + cyclic,
                [, 'b'].findIndex((x) => x === undefined), Object.keys([1, , 3]).join(),
            ].join(';');
            "#
        ),
        "6,2,4;1,2,3;0,2,4;1,2,3,4;3;16;0,a,3;1,2;true;true;true;true;3;a|b;3;2,1;1,4,5;1,9,3;1,;0;0,2"
    );
}

#[test]
fn strings_execute_natively_with_unicode_case_normalization_and_collation() {
    assert_eq!(
        result(
            r#"
            const names = ['file10.md', 'File2.md', 'résumé.md', 'apple.md'];
            const sorted = [...names].sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' }));
            const words = 'a-b-c'.split('-');
            const slug = 'Hello World'.toLowerCase().replaceAll(' ', '-');
            const greek = 'ΟΔΟΣ'.toLowerCase();
            const padded = '7'.padStart(3, '0');
            const wrapped = new String('box');
            export const result = [
                sorted.join(), words.join('+'), slug, greek, padded, 'é'.normalize('NFD').length,
                'ﬁ'.normalize('NFKC'), '  trim  '.trim(), 'abc'.at(-1), 'x'.repeat(3),
                String(42), String(Symbol('s')), typeof wrapped, wrapped + '!', 'a😀'.codePointAt(1),
                String.fromCodePoint(0x1F600).length, String.raw`a\nb`, 'straße'.toUpperCase(),
                'abc'.includes('bc'), 'abc'.startsWith('b', 1), 'a,b'.split(',', 1).length,
            ].join(';');
            "#
        ),
        "apple.md,File2.md,file10.md,résumé.md;a+b+c;hello-world;οδος;007;2;fi;trim;c;xxx;42;Symbol(s);object;box!;128512;2;a\\nb;STRASSE;true;true;1"
    );
}

#[test]
fn numbers_and_booleans_execute_natively_with_exact_formatting() {
    assert_eq!(
        result(
            r#"
            const price = 1234.5;
            export const result = [
                price.toFixed(2), (0.1 + 0.2).toPrecision(3), (255).toString(16), Number('12px'),
                Number.isInteger(5), Number.parseFloat('3.5x'), (1e21).toLocaleString(), Boolean(''),
                new Boolean(false) ? 'truthy' : 'falsy', (12345.678).toLocaleString(),
                Number.MAX_SAFE_INTEGER, (1.005).toFixed(2), typeof new Number(1), (5e-7).toExponential(1),
                Number.parseInt === parseInt, (true).toString(),
            ].join(';');
            "#
        ),
        "1234.50;0.300;ff;NaN;true;3.5;1,000,000,000,000,000,000,000;false;truthy;12,345.678;9007199254740991;1.00;object;5.0e-7;true;true"
    );
}

#[test]
fn json_and_console_execute_natively_with_callbacks_aliases_and_shadowing() {
    assert_eq!(
        result(
            r#"
        const J = JSON;
        const value = J.parse('{"a":1,"b":[2,3]}', function(key, value) {
            return key === 'a' ? value * 2 : value;
        });
        const encoded = J.stringify(value, null, 2);
        const print = console.log;
        print('%s %d %j', 'native', value.a, value.b);
        function shadow(JSON, console) { return JSON + console; }
        let failure;
        try { J.parse('[1,]'); } catch (error) { failure = error.name; }
        export const result = [encoded, shadow(2, 3), failure, Object.prototype.toString.call(J)].join('|');
    "#
        ),
        "native 2 [2,3]\n{\n  \"a\": 2,\n  \"b\": [\n    2,\n    3\n  ]\n}|5|SyntaxError|[object JSON]"
    );
    for name in ["JSON", "console"] {
        assert!(replaces_global(&format!("{name} = {{}};"), name));
    }
}

#[test]
fn tagged_templates_pass_cached_frozen_strings_raw_text_and_receivers() {
    assert_eq!(
        result(
            r#"function tag(strings, ...values) {return strings;} const sites = []; for (let index = 0; index < 2; index++) {sites.push(tag`a${index}b`);} const other = tag`a${0}b`; const strings = sites[0]; let frozen = false; try {strings[0] = 'x';} catch (error) {frozen = error instanceof TypeError;} const raw = tag`line\nbreak A ${1}`.raw; const invalid = tag`\unicode`; const receiver = {name: 'r', tag(strings, value) {return this.name + strings.join('|') + value;}}; const join = (strings, ...values) => strings.raw.join('+') + values.join(','); export const result = [sites[0] === sites[1], sites[0] !== other, strings.length, strings.join('-'), Object.isFrozen(strings), Object.isFrozen(strings.raw), frozen, raw[0], raw.length, invalid[0] === undefined, invalid.raw[0], receiver.tag`x${1}y`, join`p${'q'}r${'s'}t`, Object.getOwnPropertyDescriptor(strings, 'raw').enumerable].join('|');"#
        ),
        "true|true|2|a-b|true|true|true|line\\nbreak A |2|true|\\unicode|rx|y1|p+r+tq,s|false"
    );
}

#[test]
fn object_builtins_execute_natively_with_descriptors_integrity_and_boxing() {
    assert_eq!(
        result(
            "const o = {a: 1, b: 2}; Object.defineProperty(o, 'c', {value: 3}); const frozen = Object.freeze({x: 1}); let threw = false; try {frozen.x = 2;} catch (error) {threw = error instanceof TypeError;} const proto = {greet() {return 'hi';}}; const child = Object.create(proto); const list = [1, 2]; Object.defineProperty(list, 'length', {writable: false}); let fixed = false; try {list.push(3);} catch (error) {fixed = error instanceof TypeError;} export const result = [Object.keys(o).join(','), o.c, Object.isFrozen(frozen), threw, child.greet(), Object.getPrototypeOf(child) === proto, Object.entries({p: 1}).join(';'), Object.assign({}, o, {d: 4}).d, Object.fromEntries([['k', 'v']]).k, Object.is(NaN, NaN), typeof Object('s'), Object.getOwnPropertyNames('ab').join(','), Object.hasOwn(o, 'a'), ({}).toString.call([]), Object.groupBy([1, 2, 3], (v) => v % 2 ? 'odd' : 'even').odd.join(''), fixed, child.__proto__ === proto].join('|');"
        ),
        "a,b|3|true|true|hi|true|p,1|4|v|true|object|0,1,length|true|[object Array]|13|true|true"
    );
    assert!(replaces_global("Object = {};", "Object"));
}

#[test]
fn source_symbol_protocols_drive_iteration_conversion_tags_and_custom_instance_checks() {
    assert_eq!(
        result(
            "let closed = 0; const iterable = {[Symbol.iterator]() {let next=0;return {next() {next++;return {value:next,done:next>3};},return() {closed++;return {};}};}}; const spread = [...iterable].join(','); for(const value of iterable) {break;} const aggregate = AggregateError(iterable); let hint = ''; const number = {[Symbol.toPrimitive](value) {hint += value + ',';return 7;}}; const sum = number + 1; const converted = +number; const data = {[number]:9}; const tagged = {[Symbol.toStringTag]:'Native'}; const matcher = {[Symbol.hasInstance](value) {return value === 42;}}; const toString = ({}).toString; const symbol = Symbol('tag'); const defaultTag = toString.call(symbol); delete Symbol.prototype[Symbol.toStringTag]; const removedTag = toString.call(symbol); Symbol.prototype[Symbol.toStringTag] = 'Changed'; const changedTag = toString.call(symbol); let invalid = false; try {Symbol.keyFor({});} catch(error) {invalid=error instanceof TypeError;} let string = false; try {'' + symbol;} catch(error) {string=error instanceof TypeError;} export const result = [spread,closed,aggregate.errors.join(','),sum,converted,data[7],hint,tagged.toString(),42 instanceof matcher,43 instanceof matcher,defaultTag,removedTag,changedTag,invalid,string].join('|');"
        ),
        "1,2,3|1|1,2,3|8|7|9|default,number,string,|[object Native]|true|false|[object Symbol]|[object Object]|[object Changed]|true|true"
    );
}

#[test]
fn numeric_globals_execute_native_coercions_prefixes_and_preserve_shadowing() {
    assert_eq!(
        result(
            "const alias = parseInt; alias.tag = 7; function shadow(parseInt) {return parseInt();} let log = ''; const input = {toString() { log += 's'; return '17tail'; }}; const base = {valueOf() {log += 'r'; return 8;}}; const parsed = alias(input,base); let caught; try {parseInt('', {valueOf() {throw 'radix';}});} catch(error) {caught=error;} let constructor = false; try {new parseFloat('1');} catch(error) {constructor=error.name === 'TypeError';} export const result = [parsed,log,caught,shadow(() => 44),alias === parseInt,parseInt.tag,parseFloat('1e+'),parseFloat('-.5e2tail'),1/parseInt('-0'),1/parseFloat('-1e-400'),isNaN('x'),isNaN(),isFinite('42'),isFinite(),constructor].join('|');"
        ),
        "15|sr|radix|44|true|7|1|-50|-Infinity|-Infinity|true|true|true|false|true"
    );
    for name in ["parseInt", "parseFloat", "isNaN", "isFinite"] {
        assert!(replaces_global(&format!("{name} = 1;"), name));
    }
    assert_eq!(
        project_result(&[
            (
                "main.ts",
                "import {parseFloat, isNaN} from 'values.ts'; const isFinite = () => 3; export const result = [parseFloat(),isNaN(),isFinite()].join('|');"
            ),
            (
                "values.ts",
                "export const parseFloat = () => 1; export function isNaN() {return 2;}"
            ),
        ]),
        "1|2|3"
    );
}

#[test]
fn numeric_parsing_rounds_long_radix_integers_once_in_generated_programs() {
    let sticky = format!("1{}1{}1", "0".repeat(52), "0".repeat(106));
    let overflow = format!("1{}", "0".repeat(1024));
    let source = format!(
        "export const result = [parseInt('{sticky}',2) === 1.4615016373309032e48, +'0b{sticky}' === 1.4615016373309032e48, parseInt('{overflow}',2) === Infinity, parseInt('zZ',36), parseInt('0x10',10), parseInt('17',4294967304), parseFloat('0x10')].join('|');"
    );
    assert_eq!(result(&source), "true|true|true|1295|0|15|0");
}

#[test]
fn math_intrinsics_execute_native_numbers_and_preserve_shadowing_and_mutation() {
    assert_eq!(
        result(
            "const alias = Math; function shadow(Math) { return Math.floor(1); } const before = Math.floor(3.7); const saved = () => Math.floor(3.7); Math.floor = () => 21; const random = Math.random(); let readonly = false; try { Math.PI = 0; } catch (error) { readonly = error.name === 'TypeError'; } let constructor = false; try { new Math.sqrt(4); } catch (error) { constructor = error.name === 'TypeError'; } export const result = [typeof Math, before, saved(), alias === Math, shadow({floor: () => 44}), 1 / Math.round(-0.5), Math.sqrt('9'), Math.imul(4294967295,5), Math.f16round(1.0004882812500002), Math.sumPrecise([1e16,1,-1e16]), readonly, constructor, random >= 0 && random < 1, Math.toString()].join('|');"
        ),
        "object|3|21|true|44|-Infinity|3|-5|1.0009765625|1|true|true|true|[object Math]"
    );
    assert!(replaces_global("Math = {};", "Math"));
}

#[test]
fn math_coercion_and_precise_summation_keep_exception_order_and_iterator_close() {
    assert_eq!(
        execute(
            "export function verify(iteratorKey) {let conversions = ''; const a = {valueOf() { conversions += 'a'; return NaN; }}; const b = {valueOf() { conversions += 'b'; throw 'later'; }}; let caught; try { Math.max(a, b); } catch (error) { caught = error; } let closed = 0; const iterator = {next() {return {done:false,value:'1'};}, return() {closed++; throw 'close';}}; const values = {[iteratorKey]() {return iterator;}}; let type = false; try { Math.sumPrecise(values); } catch (error) {type = error.name === 'TypeError';} let missing = false; try { Math.sumPrecise(); } catch(error) {missing = error.name === 'TypeError';} return [conversions,caught,closed,type,missing,1/Math.sumPrecise([]),1/Math.sumPrecise([-0,-0]),Math.hypot(3e200,4e200)/1e200].join('|');}",
            "let function = ops::get(&mut realm, &exports, &Key::name(\"verify\")).unwrap(); let key = Value::Symbol(realm.symbols.iterator.clone()); let value = ops::call(&mut realm, &function, Value::Undefined, &[key]).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "ab|later|1|true|true|-Infinity|-Infinity|5"
    );
    assert_eq!(
        execute(
            "export function verify(iteratorKey) {const iterator = {[iteratorKey]() {return this;}, next() {throw 'step';}, return() {throw 'unexpected close';}}; let caught; try { Math.sumPrecise(iterator); } catch(error) {caught=error;} return caught;}",
            "let function = ops::get(&mut realm, &exports, &Key::name(\"verify\")).unwrap(); let key = Value::Symbol(realm.symbols.iterator.clone()); let value = ops::call(&mut realm, &function, Value::Undefined, &[key]).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "step"
    );
}

#[test]
fn json_dependencies_construct_native_data_and_share_cached_es_default_identity() {
    let module = mixed("main.ts", &[
        ("main.ts", "import value from 'data.json'; import same from 'entry.cjs'; export const result = [value === same, value.answer, value.name, value.items.join(','), value.__proto__.own, value.hasOwnProperty('__proto__'), value.toString(), value.big === Infinity, 1 / value.zero === -Infinity, value.tiny, value.duplicate].join('|');"),
        ("entry.cjs", "'use strict'; const value = require('data.json'); value.answer++; module.exports = require('data.json');"),
        ("data.json", "\u{feff} \r\n {\"answer\":41, \"name\":\"資料📒\", \"items\":[true,false,null,1.25e+2], \"__proto__\": {\"old\": 1}, \"__proto__\":{\"own\":7}, \"big\":1e400, \"zero\":-0, \"tiny\":1E-400, \"duplicate\":1, \"duplicate\":2} \t\n"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "true|42|資料📒|true,false,,125|7|true|[object Object]|true|true|0|2"
    );
    // JSON primitive roots, empties, numeric grammar and UTF-16 escapes retain
    // their data semantics without ever evaluating a source expression.
    let data = r#"[{}, [], "\"\\\/\b\f\n\r\t\u0000\uD800\uDC00\uDFFF", {"\uD800":"x"}, 0, -12.5e-1, 1E2, 1e999999999999999999999, "  "]"#;
    let module = mixed("root.json", &[("root.json", data)]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); let text = ops::get(&mut realm, &value, &Key::Index(2)).unwrap(); let Value::String(text) = text else { panic!(\"string\") }; let object = ops::get(&mut realm, &value, &Key::Index(3)).unwrap(); let key = Key::from_js(&nativeboat_native::JsStr::from_units(&[0xd800])); let field = ops::get(&mut realm, &object, &key).unwrap(); let negative = ops::get(&mut realm, &value, &Key::Index(5)).unwrap(); let upper = ops::get(&mut realm, &value, &Key::Index(6)).unwrap(); let huge = ops::get(&mut realm, &value, &Key::Index(7)).unwrap(); println!(\"{:?}|{}|{}|{}|{}\", text.units(), show(&mut realm, &field), show(&mut realm, &negative), show(&mut realm, &upper), show(&mut realm, &huge));"
        ),
        "[34, 92, 47, 8, 12, 10, 13, 9, 0, 55296, 56320, 57343]|x|-1.25|100|Infinity"
    );
    for (source, expected) in [
        ("true", "true"),
        ("false", "false"),
        ("null", "null"),
        ("\"text\"", "text"),
        ("2", "2"),
    ] {
        let module = mixed("root.json", &[("root.json", source)]).unwrap();
        assert_eq!(
            run(
                &module,
                "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
            ),
            expected
        );
    }
    let nested = format!("{}0{}", "[".repeat(64), "]".repeat(64));
    let module = mixed("root.json", &[("root.json", &nested)]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "0"
    );
}

#[test]
fn json_errors_retain_source_locations_and_do_not_stop_other_dependencies() {
    for source in [
        "",
        " ",
        "tru",
        "fals",
        "nul",
        "undefined",
        "NaN",
        "Infinity",
        "01",
        "+1",
        "-",
        "-.2",
        "1.",
        "1e",
        "1e+",
        "1E-",
        "0x1",
        "'text'",
        "\"\\x00\"",
        "\"\\é\"",
        "\"\\u00é0\"",
        "\"\\u012\"",
        "\"unterminated",
        "\"line\nfeed\"",
        "\"escape\\",
        "[1,]",
        "[1 2]",
        "[1",
        "{name:1}",
        "{\"a\" 1}",
        "{\"a\":1,}",
        "{\"a\":1 \"b\":2}",
        "{\"a\":1",
        "/*comment*/0",
        "1; alert()",
        "\u{feff}\u{feff}0",
        "\u{00a0}0",
    ] {
        let errors = mixed("bad.json", &[("bad.json", source)]).unwrap_err();
        assert!(
            errors
                .iter()
                .all(|error| error.path == Path::new("bad.json")
                    && error.message.starts_with("invalid JSON:")),
            "{source:?}: {errors:#?}"
        );
    }
    let deep = format!("{}{}", "[".repeat(65), "]".repeat(65));
    assert!(
        mixed("deep.json", &[("deep.json", &deep)]).unwrap_err()[0]
            .message
            .contains("nesting beyond 64")
    );
    let original = "\u{feff}{\n  \"資料\": 1,\n  \"next\": @\n}";
    let errors = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import value from 'bad.json'; import 'dep.cjs'; unknown(value);",
            ),
            ("bad.json", original),
            ("dep.cjs", "'use strict'; nestedMissing();"),
        ],
    )
    .unwrap_err();
    let error = errors
        .iter()
        .find(|error| error.path == Path::new("bad.json"))
        .unwrap();
    assert_eq!((error.line, error.column), (3, 11));
    assert_eq!(
        &original[error.span.start as usize..error.span.end as usize],
        "@"
    );
    assert!(
        !errors
            .iter()
            .any(|error| error.message.contains("global 'unknown'"))
    );
    assert!(
        !errors
            .iter()
            .any(|error| error.message.contains("global 'nestedMissing'"))
    );
    for import in [
        "import {answer} from 'data.json'; answer;",
        "import * as data from 'data.json'; data;",
        "export * from 'data.json';",
    ] {
        assert!(
            mixed(
                "main.ts",
                &[("main.ts", import), ("data.json", "{\"answer\":42}")]
            )
            .is_err()
        );
    }
    for import in [
        "import data from 'data.json' with {type:'javascript'}; data;",
        "import data from 'data.js' with {type:'json'}; data;",
        "import data from 'data.json' with {type:'json', mode:'strict'}; data;",
    ] {
        assert!(
            mixed("main.ts", &[("main.ts", import), ("data.json", "{}")])
                .unwrap_err()
                .iter()
                .any(|error| error.message.contains("import attributes"))
        );
    }
}

#[test]
fn json_import_attributes_accept_only_the_compiled_json_type() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import data from 'data.json' with {type:'json'}; export const result = data.answer;",
            ),
            ("data.json", "{\"answer\":42}"),
        ],
    )
    .unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "42"
    );

    let import = "export {default} from 'data.json' with {type:'json'};";
    let compiled = mixed("main.ts", &[("main.ts", import), ("data.json", "{}")]);
    assert!(compiled.is_ok(), "{import}: {:#?}", compiled.err());
    let import = "export {default} from 'data.json' with {'type':'json'};";
    let compiled = mixed("main.ts", &[("main.ts", import), ("data.json", "{}")]);
    assert!(compiled.is_ok(), "{import}: {:#?}", compiled.err());
}

#[test]
fn commonjs_resolve_returns_native_paths_without_loading_or_running_targets() {
    let files = std::collections::HashMap::from([
        (PathBuf::from("main.ts"), ModuleSource { text: "import value from 'dep.cjs'; export const result = [value.path, value.lazy(), value.shadow, value.value(), ...value.core].join('|');".into(), kind: ModuleKind::EsModule }),
        (PathBuf::from("dep.cjs"), ModuleSource { text: "'use strict'; function shadow(require) { return require.resolve('local'); } exports.shadow = shadow({resolve: x => 'shadow:' + x}); exports.path = require.resolve('asset.json'); exports.lazy = () => (require).resolve('esm.mjs'); exports.value = () => require('value.cjs'); exports.core = [require.resolve('path'), require.resolve('node:fs')];".into(), kind: ModuleKind::CommonJs }),
        (PathBuf::from("value.cjs"), ModuleSource { text: "'use strict'; module.exports = 7;".into(), kind: ModuleKind::CommonJs }),
    ]);
    let asset = PathBuf::from("C:\\native test\\資料📒\\asset.json");
    let esm = PathBuf::from("C:\\native test\\unevaluated.mjs");
    let mut loaded = Vec::new();
    let mut resolved = Vec::new();
    let module = compile_sources(
        Path::new("main.ts"),
        &mut |from, name, kind| {
            resolved.push((from.to_path_buf(), name.to_owned(), kind));
            Some(match name {
                "asset.json" => asset.clone(),
                "esm.mjs" => esm.clone(),
                _ => name.into(),
            })
        },
        &mut |path| {
            loaded.push(path.to_path_buf());
            Some(
                files
                    .get(path)
                    .expect("resolution-only files must never be loaded")
                    .clone(),
            )
        },
    )
    .unwrap();
    loaded.sort();
    assert_eq!(
        loaded,
        ["dep.cjs", "main.ts", "value.cjs"].map(PathBuf::from)
    );
    assert_eq!(resolved.len(), 6);
    for (from, name, kind) in resolved {
        assert_eq!(
            kind,
            if name == "dep.cjs" {
                RequestKind::Import
            } else if name == "value.cjs" {
                RequestKind::Require
            } else {
                RequestKind::Resolve
            }
        );
        assert_eq!(
            from,
            PathBuf::from(if name == "dep.cjs" {
                "main.ts"
            } else {
                "dep.cjs"
            })
        );
    }
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        format!(
            "{}|{}|shadow:local|7|path|node:fs",
            asset.display(),
            esm.display()
        )
    );
}

#[cfg(windows)]
#[test]
fn commonjs_physical_paths_preserve_windows_utf16_surrogates() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let directory = vec![67, 58, 92, 0xd800];
    let mut filename = directory.clone();
    filename.extend("\\entry.cjs".encode_utf16());
    let entry = PathBuf::from(std::ffi::OsString::from_wide(&filename));
    let target = PathBuf::from(std::ffi::OsString::from_wide(&[67, 58, 92, 0xdfff]));
    let module = compile_sources(
        &entry,
        &mut |_, _, kind| { assert_eq!(kind, RequestKind::Resolve); Some(target.clone()) },
        &mut |path| { assert_eq!(path, entry); Some(ModuleSource { text: "'use strict'; module.exports = [__filename, __dirname, require.resolve('asset')];".into(), kind: ModuleKind::CommonJs }) },
    ).unwrap();
    let expected = format!(
        "{:?}|{:?}|{:?}",
        filename,
        directory,
        target.as_os_str().encode_wide().collect::<Vec<_>>()
    );
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); let mut parts = Vec::new(); for index in 0..3 { let item = ops::get(&mut realm, &value, &Key::Index(index)).unwrap(); let Value::String(text) = item else { panic!(\"path string\") }; parts.push(format!(\"{:?}\", text.units())); } println!(\"{}\", parts.join(\"|\"));"
        ),
        expected
    );
}

#[test]
fn typescript_modules_receive_their_physical_module_path_globals() {
    let entry = std::env::temp_dir()
        .join("nativeboat")
        .join("packages")
        .join("desktop")
        .join("src")
        .join("main")
        .join("config.ts");
    let module = compile_sources(&entry, &mut |_, _, _| None, &mut |path| {
        (path == entry).then(|| ModuleSource {
            text: "export const result = [__filename, __dirname].join('|');".into(),
            kind: ModuleKind::EsModule,
        })
    })
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let expected = format!(
        "{}|{}",
        entry.to_string_lossy(),
        entry.parent().unwrap().to_string_lossy()
    );
    assert_eq!(
        run(
            &module,
            "let result = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &result));"
        ),
        expected
    );
}

#[test]
fn commonjs_resolve_rejects_dynamic_operations_and_preserves_error_discovery() {
    for expression in [
        "require.resolve()",
        "require.resolve('x', {paths: []})",
        "require.resolve?.('x')",
        "require?.resolve('x')",
        "require['resolve']('x')",
        "require.resolve",
        "require.resolve.paths('x')",
        "delete require.resolve",
        "require.resolve(...['x'])",
        "require.resolve('\\uD800')",
    ] {
        let source = format!("'use strict'; {expression};");
        let errors = mixed("entry.cjs", &[("entry.cjs", &source)]).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("require.resolve needs")
                    || error.message.contains("require properties")),
            "{expression}: {errors:#?}"
        );
    }
    let original = "'use strict';\nconst α = 1;\nrequire.resolve('unresolved.asset');\nrequire('dep.cjs');\nunknown();";
    let errors = mixed(
        "entry.cjs",
        &[
            ("entry.cjs", original),
            ("dep.cjs", "'use strict'; nestedMissing();"),
        ],
    )
    .unwrap_err();
    let missing = errors
        .iter()
        .find(|error| error.message == "cannot resolve module 'unresolved.asset'")
        .unwrap();
    assert_eq!((missing.line, missing.column), (3, 17));
    assert_eq!(
        &original[missing.span.start as usize..missing.span.end as usize],
        "'unresolved.asset'"
    );
    assert!(
        !errors
            .iter()
            .any(|error| error.message.contains("global 'nestedMissing'"))
    );
    assert!(
        !errors
            .iter()
            .any(|error| error.message.contains("global 'unknown'"))
    );
}

#[test]
fn strict_function_arguments_are_unmapped_iterable_and_arrow_captured() {
    assert_eq!(
        result(
            "export function first(a, b) { arguments[0] = 9; b = 8; let seen = []; for (const value of arguments) seen.push(value); const arrow = () => () => arguments; const args = arrow()(); return [a, b, args[0], args[1], arguments.length, seen.join(',')].join('|'); } export const result = first(1, 2, 3);"
        ),
        "1|8|9|2|3|9,2,3"
    );
    assert_eq!(
        result(
            "function f(a = arguments.length, ...rest) { const keep = () => arguments; delete arguments[0]; arguments.length = 3; arguments[2] = 7; return [a, rest.length, [...arguments].join(','), keep() === arguments].join('|'); } export const result = f();"
        ),
        "0|0|,,7|true"
    );
    assert_eq!(
        result(
            "function outer(a) { const same = () => arguments; function inner(b) { return [arguments[0], arguments.length]; } let blocked = false; try { arguments.callee; } catch (error) { blocked = error.name === 'TypeError'; } return [same()[0], inner(4,5).join(','), blocked, arguments.toString()].join('|'); } export const result = outer(3);"
        ),
        "3|4,2|true|[object Arguments]"
    );
    assert_eq!(
        result(
            "function outer(...rest) { return () => () => arguments[0]; } const saved = outer(12); export const result = saved()();"
        ),
        "12"
    );
    let top_level_arguments =
        compile_dynamic("const a = () => arguments;", Path::new("main.ts")).unwrap();
    assert!(
        top_level_arguments
            .rust_source()
            .contains("ops::get_global(realm, \"arguments\")")
    );
    assert!(compile_dynamic("function f() { arguments = []; }", Path::new("strict.ts")).is_err());
    // Strict dependency functions have arguments; the CommonJS factory's own
    // wrapper arguments cannot expose require/module past static validation.
    let module = mixed("main.ts", &[
        ("main.ts", "import f from 'dep.cjs'; export const result = f(2,3);"),
        ("dep.cjs", "'use strict'; module.exports = function(a) { return arguments[0] + arguments[1]; };"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "5"
    );
    for source in [
        "'use strict'; arguments[1]('hidden');",
        "'use strict'; const load = () => arguments[1];",
    ] {
        assert!(
            mixed("dep.cjs", &[("dep.cjs", source)])
                .unwrap_err()
                .iter()
                .any(|error| error.message.contains("wrapper arguments access"))
        );
    }
}

#[test]
fn sloppy_commonjs_maps_arguments_to_simple_parameters() {
    let module = mixed(
        "entry.cjs",
        &[(
            "entry.cjs",
            "function pair(a, b) { return a + ':' + b; } function apply(a, b) { return pair.apply(null, arguments); } function spread(a, b) { return pair(...arguments); } function inspect() { var args = arguments.length === 1 ? [arguments[0]] : Array.apply(null, arguments); return args.join(','); } function dynamicRead() { var index = 1; return arguments[index] + ':' + arguments.length; } function rest(head, ...tail) { return pair.apply(null, arguments); } function strictNested(value) { 'use strict'; value = 9; return arguments[0]; } class StrictMethod { read(value) { value = 10; return arguments[0]; } } const owner = {}; owner.on = function() { var args = arguments.length === 1 ? [arguments[0]] : Array.apply(null, arguments); args.splice(0, 0, 'event'); return args.join(','); }; function parameterWrites(value) { value = 2; return arguments[0]; } function argumentWrites(value) { arguments[0] = 3; return value; } function escaped(value) { const saved = arguments; value = 4; return saved[0]; } function deleted(value) { delete arguments[0]; value = 6; return value + ':' + arguments[0]; } function redefined(value) { Object.defineProperty(arguments, '0', { value: 8, writable: false }); value = 9; return value + ':' + arguments[0]; } module.exports = [apply(1, 2), spread(3, 4), inspect('x', 'y'), rest(5, 6), strictNested(7), new StrictMethod().read(8), owner.on('a', 'b'), dynamicRead('x', 'y'), parameterWrites(1), argumentWrites(1), escaped(1), deleted(1), redefined(1)].join('|');",
        )],
    )
    .unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "1:2|3:4|x,y|5:6|7|8|event,a,b|y:2|2|3|4|6:undefined|9:8"
    );

    for source in [
        "function callee() { return arguments.callee; } module.exports = callee();",
        "module.exports = arguments;",
        "const read = () => arguments[1]; module.exports = read();",
    ] {
        let diagnostics = mixed("blocked.cjs", &[("blocked.cjs", source)]).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|error| error.message.contains("arguments")),
            "{source}: {diagnostics:?}"
        );
    }
}

#[test]
fn function_arity_stops_at_the_first_default_for_declarations_arrows_and_methods() {
    assert_eq!(
        result(
            "function none() {} function one(a) {} function before(a, b = 1, c) {} function after(a = 1, b) {} const arrow = (a, b) => a + b; const object = { method(a, b = 1) { return a; } }; export const result = [none.length, one.length, before.length, after.length, arrow.length, object.method.length].join('|');"
        ),
        "0|1|1|0|2|1"
    );
}

#[test]
fn commonjs_cached_requires_observe_replacement_while_es_default_stays_snapshot() {
    let module = mixed("main.ts", &[
        ("main.ts", "import original from 'value.cjs'; import check from 'check.cjs'; original.replace(); export const result = [original.value, original.arrow(), check()].join('|');"),
        ("value.cjs", "'use strict'; exports.value = 3; exports.arrow = () => this === exports; exports.replace = () => { module.exports = 99; };"),
        ("check.cjs", "'use strict'; module.exports = () => require('value.cjs');"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "3|true|99"
    );
}

#[test]
fn commonjs_failure_retries_and_default_reexports_keep_source_identity() {
    let module = mixed("main.ts", &[
        ("main.ts", "export {default as result} from 'entry.cjs';"),
        ("entry.cjs", "#!native-test\n'use strict'; let first; try { require('fails.cjs'); } catch (error) { first = error; } const second = require('fails.cjs'); function shadow(require) { return require('local'); } module.exports = [first, second, shadow(x => x + '!'), __filename, __dirname].join('|');"),
        ("fails.cjs", "'use strict'; const state = require('state.cjs'); if (++state.attempt === 1) throw 'first'; module.exports = 'second';"),
        ("state.cjs", "'use strict'; module.exports = {attempt: 0};"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "first|second|local!|entry.cjs|"
    );
    let entry = mixed(
        "entry.cjs",
        &[("entry.cjs", "'use strict'; module.exports = 42;")],
    )
    .unwrap();
    assert_eq!(
        run(
            &entry,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "42"
    );
}

#[test]
fn sloppy_commonjs_functions_coerce_this_but_strict_functions_do_not() {
    let sloppy = mixed(
        "entry.cjs",
        &[("entry.cjs", "function globalReceiver() { return this === globalThis; } function boxedReceiver() { return typeof this + ':' + this.valueOf(); } const objectMethod = { receiver() { return this === globalThis; } }; const arrow = () => this === exports; class C { receiver() { return this === null; } } module.exports = [globalReceiver.call(null), boxedReceiver.call(3), objectMethod.receiver.call(null), arrow(), C.prototype.receiver.call(null)].join('|');")],
    )
    .unwrap();
    assert_eq!(
        run(
            &sloppy,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "true|object:3|false|true|true"
    );

    let strict = mixed(
        "entry.cjs",
        &[("entry.cjs", "'use strict'; function receiver() { return this === null; } module.exports = receiver.call(null);")],
    )
    .unwrap();
    assert_eq!(
        run(
            &strict,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "true"
    );
}

#[test]
fn commonjs_blockers_retain_original_unicode_positions() {
    for (source, message) in [
        (
            "'use strict'; const cache = require.cache;",
            "require properties",
        ),
        (
            "'use strict'; var require = x => x; require('x');",
            "wrapper binding replacement",
        ),
        (
            "'use strict'; function module() {}",
            "wrapper binding replacement",
        ),
        ("'use strict'; const x = ;", "Unexpected"),
        ("'use strict'; let x; let x;", "already"),
    ] {
        let diagnostics = mixed("blocked.cjs", &[("blocked.cjs", source)]).unwrap_err();
        let error = diagnostics
            .iter()
            .find(|error| error.message.contains(message))
            .unwrap_or_else(|| panic!("{source}: {diagnostics:?}"));
        assert_eq!(error.path, Path::new("blocked.cjs"));
        assert!(error.span.end as usize <= source.len());
        if message == "global 'unknown'" {
            assert_eq!((error.line, error.column), (3, 1));
            assert_eq!(
                &source[error.span.start as usize..error.span.end as usize],
                "unknown"
            );
        }
    }
    let sloppy = mixed(
        "entry.cjs",
        &[("entry.cjs", "this.value = 42; module.exports = this.value;")],
    )
    .unwrap();
    assert_eq!(
        run(
            &sloppy,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); println!(\"{}\", show(&mut realm, &value));"
        ),
        "42"
    );
    assert!(
        mixed("bad.unknown", &[("bad.unknown", "'use strict';")]).unwrap_err()[0]
            .message
            .contains("Unknown")
    );
    // Named and namespace imports of CommonJS link; star re-exports do not,
    // and JSON modules only have a default export.
    let blocked = mixed("main.ts", &[
        ("main.ts", "import {named} from 'dep.cjs'; import * as ns from 'dep.cjs'; export * from 'dep.cjs'; export * as c from 'dep.cjs'; export {named as n} from 'dep.cjs'; import {key} from 'data.json'; import * as data from 'data.json'; export * as d from 'data.json'; [named, ns, key, data];"),
        ("dep.cjs", "'use strict'; exports.named = 1;"),
        ("data.json", "{\"key\": 1}"),
    ]).unwrap_err();
    let messages: Vec<&str> = blocked.iter().map(|error| error.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "module 'data.json' has no export 'key'",
            "JSON module namespace imports are not compiled yet; only default imports are supported",
            "JSON module namespace re-export 'd' is not compiled yet",
            "CommonJS star re-export interop is not compiled yet",
        ],
        "{blocked:?}"
    );
    // `require` of an ES module gives its namespace, as Node 22 and bundlers
    // do, also through CommonJS modules other CommonJS modules require.
    let module = mixed(
        "main.ts",
        &[
            ("main.ts", "import lib from './lib.cjs'; export const result = lib;"),
            (
                "lib.cjs",
                "'use strict'; const esm = require('esm.ts'); const inner = require('inner.cjs');\n\
                 module.exports = [esm.value, esm.default, typeof esm.missing, inner, esm === require('esm.ts'), Object.prototype.toString.call(esm)].join('|');",
            ),
            ("inner.cjs", "'use strict'; module.exports = require('esm.ts').value + 1;"),
            ("esm.ts", "export const value = 1; export default 'd';"),
        ],
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(
        print_result(&module),
        "1|d|undefined|2|true|[object Module]"
    );
    let module = mixed(
        "main.cjs",
        &[
            (
                "main.cjs",
                "'use strict'; module.exports = require('esm.ts').value;",
            ),
            ("esm.ts", "export const value = 'from esm';"),
        ],
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert!(module.warnings().is_empty());
    // An unresolved target throws when it is required, with one warning and
    // no invented interop errors.
    let missing = mixed(
        "main.cjs",
        &[("main.cjs", "'use strict'; require('missing.cjs');")],
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(missing.warnings().len(), 1, "{:?}", missing.warnings());
    assert!(
        missing.warnings()[0]
            .message
            .contains("throws MODULE_NOT_FOUND when it runs")
    );
    // Requests computed when the application runs compile, with a warning,
    // and throw then as a missing module does.
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import all from './all.cjs'; export const result = all;",
            ),
            (
                "all.cjs",
                "'use strict'; const name = 'plugin' + '.cjs'; let thrown;
try { require(name); } catch (error) { thrown = error.code; }
module.exports = [thrown, require('other.cjs'), require?.('other.cjs')].join('|');",
            ),
            ("other.cjs", "'use strict'; module.exports = 'other';"),
        ],
    )
    .unwrap();
    let warnings: Vec<_> = module
        .warnings()
        .iter()
        .map(|warning| (warning.line, warning.column, warning.message.as_str()))
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert_eq!((warnings[0].0, warnings[0].1), (2, 7));
    assert!(warnings[0].2.contains("computed when the application runs"));
    assert_eq!(print_result(&module), "MODULE_NOT_FOUND|other|other");
}

#[test]
fn commonjs_require_passed_as_a_value_loads_named_modules() {
    // Browserify and UMD bundles keep `require` for modules they lack.
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import all from './bundle.cjs'; export const result = all;",
            ),
            (
                "bundle.cjs",
                "'use strict'; require('other.cjs');\n\
                 var c = 'function' == typeof require && require; var r = c; let thrown;\n\
                 try { r('missing' + ''); } catch (error) { thrown = error.code; }\n\
                 module.exports = [typeof require, r('other.cjs'), thrown].join('|');",
            ),
            ("other.cjs", "'use strict'; module.exports = 'other';"),
        ],
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let warnings: Vec<_> = module
        .warnings()
        .iter()
        .map(|warning| (warning.line, warning.column, warning.message.as_str()))
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!((warnings[0].0, warnings[0].1), (2, 41));
    assert!(warnings[0].2.contains("require is used as a value"));
    assert_eq!(print_result(&module), "function|other|MODULE_NOT_FOUND");
    // Its properties, other than `main` and `resolve`, and assignments to it
    // are not provided.
    for source in [
        "'use strict'; module.exports = require.cache;",
        "'use strict'; require['x'] = 1;",
        "require = function () {};",
    ] {
        let errors = mixed("entry.cjs", &[("entry.cjs", source)]).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("require properties and assignments")),
            "{source}: {errors:#?}"
        );
    }
}

#[test]
fn commonjs_computed_callee_is_supported_and_lexical_module_rebinding_is_diagnosed() {
    let supported = compile_sources(Path::new("supported.cjs"), &mut |_, _, _| None, &mut |_| {
        Some(ModuleSource {
            text: "module.exports = function() { return ({}).callee; };".to_owned(),
            kind: ModuleKind::CommonJs,
        })
    })
    .unwrap();
    let rust = supported.rust_source();
    assert!(
        rust.contains("ops::get_static_at(realm, &__object, &S"),
        "{rust}"
    );
    assert!(rust.contains("StaticStr::new(\"callee\")"), "{rust}");
    // The read has a cache of its own.
    assert!(rust.contains("LookupCache = LookupCache::new();"), "{rust}");

    let errors = compile_sources(Path::new("blocked.cjs"), &mut |_, _, _| None, &mut |_| {
        Some(ModuleSource {
            text: "module.exports = arguments['callee'];".to_owned(),
            kind: ModuleKind::CommonJs,
        })
    })
    .unwrap_err();
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("CommonJS wrapper arguments access is not compiled yet")
    }));
    // The parser rejects lexical redeclarations of the wrapper's `module`
    // parameter before CommonJS binding collection can encounter them.
    let error = compile_sources(Path::new("blocked.cjs"), &mut |_, _, _| None, &mut |_| {
        Some(ModuleSource {
            text: "let module; module = {};".to_owned(),
            kind: ModuleKind::CommonJs,
        })
    })
    .unwrap_err();
    assert!(
        error
            .iter()
            .any(|error| error.message.contains("already been declared"))
    );
}

#[test]
fn exported_vars_are_initialized_before_module_evaluation() {
    assert_eq!(
        project_result(&[
            (
                "main.ts",
                "import { value, absent, alias, next } from 'vars.ts'; next(); export const result = [value, absent, alias].join('|');"
            ),
            (
                "vars.ts",
                "export var value = 4; export var absent; var local = 7; export { local as alias }; export function next() { value++; }"
            ),
        ]),
        "5||7"
    );
}

#[test]
fn cyclic_imports_observe_hoisted_vars_without_resetting_writes() {
    assert_eq!(
        project_result(&[
            (
                "main.ts",
                "import { early } from 'cycle.ts'; export var value = 'ready'; absent = 'retained'; export var absent; export var initialized = 'initializer'; export const result = [early, value, absent, initialized].join('|');"
            ),
            (
                "cycle.ts",
                "import { value } from 'main.ts'; export const early = typeof value;"
            ),
        ]),
        "undefined|ready|retained|initializer"
    );
}

fn project_result(files: &[(&str, &str)]) -> String {
    let module = link(files).unwrap_or_else(|errors| panic!("{errors:#?}"));
    run(
        &module,
        "match ops::get(&mut realm, &exports, &Key::name(\"result\")) {\n\
         Ok(value) => { let text = show(&mut realm, &value); println!(\"{text}\"); }\n\
         Err(thrown) => { let text = show(&mut realm, &thrown.0); println!(\"export threw {text}\"); }\n\
         }",
    )
}

#[test]
fn inline_types_erase_bindings_and_requests_but_keep_live_value_exports() {
    assert_eq!(
        project_result(&[
            (
                "main.ts",
                r#"
                import base, { type Absent, type Shape as Alias, count, next, type } from 'value.ts';
                import { type Never } from 'unresolved-types';
                import type { Another } from 'unresolved-declaration';
                import * as ns from 'bridge.ts';
                import 'side.js';
                import {} from 'empty.js';
                export { type Alias, count as localCount };
                export { type Absent as Hidden } from 'unresolved-reexport';
                export type { Missing } from 'unresolved-export-declaration';
                next();
                const value: Alias = base + count;
                export const result = [value, count, ns.count, ns.next === next, type,
                    Object.keys(ns).join(','), globalThis.side, globalThis.empty].join('|');
            "#
            ),
            (
                "value.ts",
                "export default 40; export let count = 0; export function next() { count++; } export const type = 'value named type';"
            ),
            (
                "bridge.ts",
                "export { type Absent as Hidden, count, next } from 'value.ts'; export {} from 'empty-reexport.js';"
            ),
            ("side.js", "globalThis.side = 'side effect';"),
            ("empty.js", "globalThis.empty = 'empty import';"),
            ("empty-reexport.js", "globalThis.empty = 'empty reexport';"),
        ]),
        "41|1|1|true|value named type|count,next|side effect|empty import"
    );
}

#[test]
fn inline_types_do_not_select_native_dependencies_or_hide_value_diagnostics() {
    let module = mixed("main.ts", &[
        ("main.ts", "import { type Absent } from 'types.native'; export { type Missing } from 'other.native'; export const result = 1;"),
        ("types.native", "binding/uncompiled"),
        ("other.native", "binding/other-uncompiled"),
    ]).unwrap();
    assert!(module.native_dependencies().is_empty());
    let errors = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import { type Shape, absent } from 'value.ts';\nexport const result = absent;",
            ),
            ("value.ts", "export const present = 1;"),
        ],
    )
    .unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].line, 1);
    assert_eq!(errors[0].column, 22);
    assert_eq!(
        errors[0].message,
        "module 'value.ts' has no export 'absent'"
    );
    let errors = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "export { type Shape, absent as result } from 'value.ts';",
            ),
            ("value.ts", "export const present = 1;"),
        ],
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message == "cannot re-export 'absent' as 'result'")
    );
}

#[test]
fn modules_link_live_bindings_cycles_namespaces_and_defaults() {
    let output = project_result(&[
        (
            "main.ts",
            r#"
            import { order } from "log.ts";
            import { count, increment } from "counter.ts";
            import * as counter from "counter.ts";
            import anonymous, { named as renamed } from "defaults.ts";
            import value from "expression.ts";
            import declared from "declared.ts";
            import { again, star, all } from "reexports.ts";
            import { early } from "cycle-a.ts";
            import type { Unused } from "types.ts";
            order.push("main");
            const log = [count];
            increment();
            log.push(count, counter.count, anonymous(), anonymous.name, renamed, value, declared(),
              declared.name, again, star, all.count, early(), typeof counter.missing);
            try { count = 5; } catch (error) { log.push(error.message); }
            export const result = order.join(">") + " | " + log.join(" ");
            "#,
        ),
        ("log.ts", "export const order = [];"),
        (
            "counter.ts",
            r#"import { order } from "log.ts"; order.push("counter");
            export let count = 0; export function increment() { count++; }"#,
        ),
        (
            "defaults.ts",
            r#"export default function () { return "anonymous"; } export const named = "n";"#,
        ),
        ("expression.ts", "export default 40 + 2;"),
        (
            "declared.ts",
            "export default function declared() { return 'declared'; }",
        ),
        (
            "reexports.ts",
            r#"export { count as again } from "counter.ts"; export * from "star.ts";
            export * as all from "counter.ts"; export * from "reexports.ts";"#,
        ),
        (
            "star.ts",
            r#"export const star = "s"; export default "not re-exported";"#,
        ),
        (
            "cycle-a.ts",
            r#"import { order } from "log.ts"; import { b } from "cycle-b.ts"; order.push("a");
            export const a = "A"; export function early() { return b(); }"#,
        ),
        (
            "cycle-b.ts",
            r#"import { order } from "log.ts"; import { a } from "cycle-a.ts"; order.push("b");
            export function b() { return a; }"#,
        ),
    ]);
    assert_eq!(
        output,
        "counter>b>a>main | 0 1 1 anonymous default n 42 declared declared 1 s 1 A undefined Assignment to constant variable."
    );
}

#[test]
fn cyclic_imports_observe_the_temporal_dead_zone() {
    let output = project_result(&[
        (
            "main.ts",
            r#"import { b } from "b.ts"; export const a = "A"; export const result = b;"#,
        ),
        (
            "b.ts",
            r#"import { a } from "main.ts"; export const b = a;"#,
        ),
    ]);
    assert_eq!(
        output,
        "module threw ReferenceError: Cannot access 'a' before initialization"
    );
}

#[test]
fn linking_errors_are_reported_with_every_other_blocker() {
    let errors = link(&[
        (
            "main.ts",
            r#"
            import { absent } from "plain.ts";
            import { x } from "ambiguous.ts";
            import gone from "missing.ts";
            import unknown from "nowhere";
            import { again } from "nowhere";
            import "broken.ts";
            export { nothing } from "plain.ts";
            class Blocked { accessor x = 1; }
            import { looped } from "loop-a.ts";
            import starredDefault from "ambiguous.ts";
            import { x as nested } from "nested.ts";
            import { present } from "mixed.ts";
            import "class-default.ts";
            import "interface-default.ts";
            [absent, x, gone, unknown, again, looped, starredDefault, nested, present];
            "#,
        ),
        ("loop-a.ts", r#"export { looped } from "loop-b.ts";"#),
        ("loop-b.ts", r#"export { looped } from "loop-a.ts";"#),
        ("nested.ts", r#"export * from "ambiguous.ts";"#),
        (
            "mixed.ts",
            r#"export * from "one.ts"; export * from "plain.ts";"#,
        ),
        (
            "class-default.ts",
            "export default class { accessor y = 1; }",
        ),
        (
            "interface-default.ts",
            "export default interface Shape { size: number }",
        ),
        ("plain.ts", "export const present = 1;"),
        (
            "ambiguous.ts",
            r#"export * from "one.ts"; export * from "two.ts";"#,
        ),
        ("one.ts", "export const x = 1;"),
        ("two.ts", "export const x = 2;"),
        ("broken.ts", "let = ;"),
    ])
    .unwrap_err();
    for expected in [
        "main.ts:2:22: module 'plain.ts' has no export 'absent'",
        "main.ts:3:22: export 'x' of module 'ambiguous.ts' is ambiguous",
        "main.ts:4:30: cannot read module 'missing.ts'",
        "main.ts:5:33: cannot resolve module 'nowhere'",
        "main.ts:6:35: cannot resolve module 'nowhere'",
        "main.ts:8:22: cannot re-export 'nothing' as 'nothing'",
        "main.ts:9:29: auto-accessors are not compiled yet",
        "main.ts:10:22: module 'loop-a.ts' has no export 'looped'",
        "main.ts:11:20: module 'ambiguous.ts' has no export 'default'",
        "main.ts:12:22: export 'x' of module 'nested.ts' is ambiguous",
        "class-default.ts:1:24: auto-accessors are not compiled yet",
        "loop-a.ts:1:10: cannot re-export 'looped' as 'looped'",
    ] {
        assert!(
            errors.iter().any(|error| error == expected),
            "missing {expected:?} in {errors:#?}"
        );
    }
    assert!(
        errors.iter().any(|error| error.starts_with("broken.ts:1:")),
        "{errors:#?}"
    );
    let unreadable =
        compile_project(Path::new("absent.ts"), &mut |_, _| None, &mut |_| None).unwrap_err();
    assert_eq!(
        unreadable[0].to_string(),
        "absent.ts:1:1: cannot read the module"
    );
}

#[test]
fn destructuring_spread_and_rest() {
    assert_eq!(
        project_result(&[
            (
                "main.ts",
                "import { head, tail, x, remaining } from 'pattern.ts'; export const result = [head, tail.join('+'), x, remaining.y].join(' ');"
            ),
            (
                "pattern.ts",
                "export const [head, ...tail] = [3, 4, 5]; export const { x, ...remaining } = { x: 6, y: 7 };"
            ),
        ]),
        "3 4+5 6 7"
    );
    let output = result(
        r#"
        const log = [];
        const [a, , b = 3, ...rest] = [1, 2, undefined, 4, 5];
        const { x, y: { z = 9 } = {}, ["comp" + "uted"]: computed, ...others } =
          { x: 1, y: {}, computed: "c", p: 2, q: 3 };
        log.push(a, b, rest.join("+"), x, z, computed, others.p + others.q, "x" in others);
        function params({ m, n = 2 }, [first, ...more] = [7, 8], ...tail) {
          return [m, n, first, more.length, tail.length].join(":");
        }
        log.push(params({ m: 1 }), params({ m: 1, n: 5 }, [0], 9, 9), params.length);
        let left = "L", right = "R", missing, deep;
        [left, right] = [right, left];
        const box = { list: [] };
        [box.first, box.list[0] = "d", ...box.others] = ["f", undefined, "o1", "o2"];
        ({ left: box.renamed, missing = "dm", ...box.remaining } = { left: "x", extra: "e" });
        const nested = ([[deep]] = [[ "deep" ]]);
        log.push(left + right, box.first, box.list[0], box.others.length, box.renamed, missing,
          box.remaining.extra, deep, nested.length);
        var [declared] = ["var"];
        log.push(declared);
        const merged = { ...{ a: 1, b: 1 }, b: 2, ..."hi", ...null };
        log.push(merged.a + merged.b, merged[0] + merged[1]);
        function sum(...values) { let total = 0; for (const value of values) total += value; return total; }
        log.push(sum(...[1, 2], 3, ..."45"), [...[1, 2], ..."ab"].join(""));
        const holes = [1, , 3];
        log.push(holes.length, 1 in holes, holes.join("-"));
        try { const [] = null; } catch ({ message }) { log.push(message); }
        try { const { q } = undefined; } catch (error) { log.push(error.message); }
        export const result = log.join(" ");
        "#,
    );
    assert_eq!(
        output,
        "1 3 4+5 1 9 c 5 false 1:2:7:1:0 1:5:0:0:2 1 RL f d 2 x dm e deep 1 var 3 hi 645 12ab 3 false 1--3 value is not iterable Cannot destructure a null or undefined value"
    );
}

#[test]
fn loops_over_iterables_keys_and_cases() {
    let output = result(
        r#"
        const log = [];
        for (const [key, value] of [["a", 1], ["b", 2]]) log.push(key + value);
        for (const character of "a😀") log.push(character.length);
        const target = {};
        let last;
        for (last of [1, 2]) {}
        for (target.item of ["member"]) {}
        var seen;
        for (seen of "z") {}
        log.push(last, target.item, seen);
        const object = { p: 1, q: 2, r: 3 };
        for (const key in object) { if (key === "p") delete object.q; log.push(key); }
        for (const index in "ab") log.push(index);
        for (var name in null) log.push("never");
        for (target.key in { only: 1 }) {}
        log.push(target.key);
        const closures = [];
        for (let i = 0; i < 3; i++) { closures.push(() => i); if (i === 1) continue; }
        for (let j = 0, k = 10; j < 2; j++, k--) closures.push(() => j * k);
        for (let once = 0; ; once++) { closures.push(() => once); break; }
        for (const fn of closures) log.push(fn());
        const later = [];
        for (const item of [1, 2]) later.push(() => item);
        log.push(later[0]() + later[1]());
        function classify(value) {
          switch (value) {
            case 0: return "zero";
            default: return "other";
            case 1:
            case 2: { let local = "small"; return local; }
            case "s": const shared = "s"; return shared + "!";
          }
        }
        log.push(classify(0), classify(2), classify("s"), classify(9));
        let fall = "";
        switch (3) { case 3: fall += "3"; case 4: fall += "4"; break; case 5: fall += "5"; }
        outer: for (const n of [1, 2, 3]) {
          switch (n) { case 2: continue outer; case 3: break outer; }
          fall += n;
        }
        named: switch (1) { case 1: break named; }
        log.push(fall);
        export const result = log.join(" ");
        "#,
    );
    assert_eq!(
        output,
        "a1 b2 1 2 2 member z p r 0 1 only 0 1 2 0 9 0 3 zero small s! other 341"
    );
}

#[test]
fn binding_lifetimes_and_pattern_order_execute_natively() {
    let output = result(
        r#"
        const log = [];
        try { const [a = b, b = 2] = []; } catch (error) { log.push(error.name); }
        try { const [a = (a = 1)] = []; } catch (error) { log.push(error.name); }
        function defaults(a = b, b = 1) { return a; }
        function self(a = a) { return a; }
        function prior(a = 2, b = a) { return b; }
        try { defaults(); } catch (error) { log.push(error.name); }
        try { self(); } catch (error) { log.push(error.name); }
        log.push(prior());
        try { for (const item of [item]) {} } catch (error) { log.push(error.name); }
        try { for (let key in key) {} } catch (error) { log.push(error.name); }
        const callbacks = [], init = [], update = [];
        for (let i = (init.push(() => i), 0); i < 2; update.push(() => i), i++) {
          callbacks.push(() => i);
        }
        log.push(init[0](), callbacks[0](), callbacks[1](), update[0](), update[1]());
        for (const one = 0; one < 1;) { log.push((() => one)()); break; }
        const noUpdate = [];
        for (let i = 0; i < 2;) { noUpdate.push(() => i); i++; }
        log.push(noUpdate[0](), noUpdate[1]());
        const keys = [];
        for (const key in { a: 1, b: 2 }) keys.push(() => key);
        log.push(keys[0](), keys[1]());
        const outer = 4;
        switch (outer) { case 4: function twice(x) { return x * 2; } log.push(twice(outer)); }
        switch (1) { case hoisted(): function hoisted() { return 1; } break; }
        try { switch (1) { case 1: log.push(later); break; case 2: let later = 2; } }
        catch (error) { log.push(error.name); }
        try { switch (1) { case later: break; default: let later; } }
        catch (error) { log.push(error.name); }
        const holder = {}, order = [];
        function target() { order.push("target"); return holder; }
        let ignored;
        [ignored, , holder.rest] = [1, 2, 3];
        [target().value = (order.push("default"), 7)] = [undefined];
        log.push(order.join(","), holder.value, holder.rest);
        ({ ["p"]: [holder.nested], ...holder.others } = { p: [8], q: 9 });
        log.push(holder.nested, holder.others.q);
        var [v = 1, ...more] = [];
        log.push(v, more.length);
        export const result = log.join(" ");
        "#,
    );
    assert_eq!(
        output,
        "ReferenceError ReferenceError ReferenceError ReferenceError 2 ReferenceError ReferenceError 0 0 1 1 2 0 1 2 a b 8 ReferenceError ReferenceError target,default 7 3 8 9 1 0"
    );
}

#[test]
fn iterators_close_when_loops_and_patterns_end_early() {
    let output = execute(
        r#"
        export function consume(iterable, mode) {
          if (mode === "break") { for (const value of iterable) break; }
          if (mode === "return") { for (const value of iterable) return value; }
          if (mode === "throw") { try { for (const value of iterable) throw "body"; } catch (error) { return error; } }
          if (mode === "continue") {
            outer: for (const tick of [1, 2]) { for (const value of iterable) continue outer; }
          }
          if (mode === "pattern") { const [first] = iterable; return first; }
          if (mode === "throwing pattern") {
            try { const [first = (() => { throw "default"; })()] = iterable; } catch (error) { return error; }
          }
          if (mode === "assignment") { let one; [one] = iterable; return one; }
          if (mode === "elision") { const [, one] = iterable; return one; }
          if (mode === "assignment elision") { let one; [, one] = iterable; return one; }
          if (mode === "exhaust") { for (const value of iterable) {} }
          return "done";
        }
        "#,
        "let consume = ops::get(&mut realm, &exports, &Key::name(\"consume\")).unwrap();\n\
         let mut lines = Vec::new();\n\
         for mode in [\"break\", \"return\", \"throw\", \"continue\", \"pattern\", \"throwing pattern\", \"assignment\", \"elision\", \"assignment elision\", \"exhaust\"] {\n\
           let closed = alloc::rc::Rc::new(core::cell::Cell::new(0));\n\
           let count = closed.clone();\n\
           let iterator = realm.object();\n\
           let steps = alloc::rc::Rc::new(core::cell::Cell::new(0));\n\
           let next = realm.function(\"next\", 0, false, move |realm, _, _| {\n\
             steps.set(steps.get() + 1);\n\
             let result = realm.object();\n\
             result.define(Key::name(\"value\"), nativeboat_native::Property::data(if mode == \"throwing pattern\" { Value::Undefined } else { Value::Number(1.0) }));\n\
             if mode.ends_with(\"elision\") && steps.get() == 1 {\n\
               let getter = realm.function(\"value\", 0, false, |realm, _, _| Err(realm.throw(nativeboat_native::ErrorKind::Error, \"must skip value\")));\n\
               ops::define_accessor(&result, Key::name(\"value\"), Some(getter), None);\n\
             }\n\
             result.define(Key::name(\"done\"), nativeboat_native::Property::data(Value::Boolean(steps.get() > 2)));\n\
             Ok(Value::Object(result))\n\
           });\n\
           let finish = realm.function(\"return\", 0, false, move |realm, _, _| { count.set(count.get() + 1); Ok(Value::Object(realm.object())) });\n\
           iterator.define(Key::name(\"next\"), nativeboat_native::Property::data(Value::Object(next)));\n\
           iterator.define(Key::name(\"return\"), nativeboat_native::Property::data(Value::Object(finish)));\n\
           let iterable = realm.object();\n\
           let source = iterator.clone();\n\
           let make = realm.function(\"iterator\", 0, false, move |_, _, _| Ok(Value::Object(source.clone())));\n\
           iterable.define(Key::Symbol(realm.symbols.iterator.clone()), nativeboat_native::Property::data(Value::Object(make)));\n\
           let result = ops::call(&mut realm, &consume, Value::Undefined, &[Value::Object(iterable), Value::str(mode)]).unwrap();\n\
           let text = show(&mut realm, &result);\n\
           lines.push(format!(\"{mode}={text}/{}\", closed.get()));\n\
         }\n\
         println!(\"{}\", lines.join(\" \"));",
    );
    assert_eq!(
        output,
        "break=done/1 return=1/1 throw=body/1 continue=done/2 pattern=1/1 throwing pattern=default/1 assignment=1/1 elision=1/1 assignment elision=1/1 exhaust=done/0"
    );
}

#[test]
fn collections_lookup_order_subclasses_and_mutation_execute_natively() {
    let output = result(
        r#"const out=[];
const key={}, sym=Symbol('key'), map=new Map([[key,'object'],[sym,'symbol'],[NaN,'first'],[-0,'zero']]);
map.set(NaN,'updated').set(0,'positive');
out.push(map.size,map.get(key),map.get(sym),map.get(NaN),map.get(-0),1/[...map.keys()][3]);
out.push(map.has({}),map.delete('absent'),map.get('absent'));
const values=map.values();out.push(values.next().value);
map.delete(sym);map.set('last','tail');out.push([...values].join(','));
map.set('after','done');out.push(values.next().done);
out.push(Map.prototype.entries===Map.prototype[Symbol.iterator]);
class Child extends Map {set(key,value){out.push('set:'+key);return super.set(key,value*2);}}
const child=new Child([['a',2]]);out.push(child.get('a'),child instanceof Child,child instanceof Map,Child[Symbol.species]===Child);
const callbackMap=new Map([['a',1],['skip',2]]);
callbackMap.forEach(function(value,key,self){out.push(this.label+key+value,self===callbackMap);if(key==='a'){self.delete('skip');self.set('b',3);}}, {label:'m'});
const set=new Set('a😀a');out.push(set.size,[...set].join(','),Set.prototype.keys===Set.prototype.values,Set.prototype.values===Set.prototype[Symbol.iterator]);
set.add(-0).add(NaN).add(NaN);out.push(set.size,set.has(0),1/[...set][2]);
const entries=[...set.entries()];out.push(entries.every(pair=>Object.is(pair[0],pair[1])));
const callbackSet=new Set(['a','skip']);callbackSet.forEach(function(value,key,self){out.push(this.label+value,key===value,self===callbackSet);if(key==='a'){self.delete('skip');self.add('b');}}, {label:'s'});
const live=callbackSet.values();out.push(live.next().value);callbackSet.clear();callbackSet.add('fresh');out.push([...live].join(','));
const frozen=Object.freeze(new Set());frozen.add(1);out.push(frozen.size);
try{Map();}catch(error){out.push(error.name);}
try{Set();}catch(error){out.push(error.name);}
try{Map.prototype.get.call(new Set(),1);}catch(error){out.push(error.name);}
try{new Map([1]);}catch(error){out.push(error.name);}
try{new Set(1);}catch(error){out.push(error.name);}
out.push(new Map(null).size,new Set(undefined).size);
export const result=out.join('|');

"#,
    );
    assert_eq!(
        output,
        "4|object|symbol|updated|positive|Infinity|false|false||object|updated,positive,tail|true|true|set:a|4|true|true|true|ma1|true|mb3|true|2|a,😀|true|true|4|true|Infinity|true|sa|true|true|sb|true|true|a|fresh|1|TypeError|TypeError|TypeError|TypeError|TypeError|0|0"
    );
}

#[test]
fn collection_construction_orders_getters_and_closes_consumers() {
    let output = result(
        r#"const out=[];
for(const C of [Map,Set]) {
 const set=C===Set;
 for(const mode of ['primitive','key','value','adder','step']) {
  if(set && (mode==='primitive'||mode==='key'||mode==='value'))continue;
  let closed=0, read=0;
  const item=mode==='primitive'?1:{get 0(){out.push('key');if(mode==='key')throw 'key';return 'k';},get 1(){out.push('value');if(mode==='value')throw 'value';return 'v';}};
  const source={[Symbol.iterator](){return {next(){if(mode==='step')throw 'step';return {value:item,done:read++>0};},return(){closed++;throw 'close';}};}};
  class Child extends C {}
  if(mode==='adder')Object.defineProperty(Child.prototype,set?'add':'set',{value(){throw 'adder';}});
  try{new Child(source);}catch(error){out.push(typeof error==='string'?error:error.name);}
  out.push(closed);
 }
 let iterated=0;
 const source={[Symbol.iterator](){iterated++;return [][Symbol.iterator]();}};
 class Child extends C {}
 Object.defineProperty(Child.prototype,set?'add':'set',{get(){out.push('adder getter');return 0;}});
 try{new Child(source);}catch(error){out.push(error.name);}
 out.push(iterated);
}
export const result=out.join('|');

"#,
    );
    assert_eq!(
        output,
        "TypeError|1|key|key|1|key|value|value|1|key|value|adder|1|step|0|adder getter|TypeError|0|adder|1|step|0|adder getter|TypeError|0"
    );
}

#[test]
fn regular_expressions_compile_to_native_matching() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        r#"
        const out = [];
        const words = /(?<word>\w+)/g;
        out.push('one two'.replace(words, '[$<word>]'));
        out.push(JSON.stringify('a1b22c333'.match(/\d+/g)));
        out.push([...'x=1;y=2'.matchAll(/(\w)=(\d)/g)].map((m) => m[1] + m[2] + m.index).join(','));
        out.push('a, b ,c'.split(/\s*,\s*/).join('|'));
        const date = /(\d{4})-(\d{2})/.exec('on 2026-10');
        out.push(date.index, date[1], date.length);
        out.push(String(/a\/b/gi), new RegExp('x', 'y').flags, RegExp('\\d').source);
        const make = () => /z/g;
        out.push(make() !== make(), /^\p{Lu}/u.test('Ábc'), '😀'.replace(/./u, 'U'));
        let caught = '';
        try { new RegExp('['); } catch (error) { caught = error.name; }
        out.push(caught);
        const sticky = /b/y;
        sticky.lastIndex = 1;
        out.push(sticky.test('abc'), sticky.lastIndex);
        out.push('Hello World'.replace(/o/g, (m, offset) => offset));
        export const result = out.join('|');
    "#,
    );
    assert_eq!(
        output,
        r#"[one] [two]|["1","22","333"]|x10,y24|a|b|c|3|2026|3|/a\/b/gi|y|\d|true|true|U|SyntaxError|true|2|Hell4 W7rld"#
    );
}

#[test]
fn invalid_regular_expression_literals_are_early_errors() {
    let errors = errors("export const pattern = /(?<a>x)(?<a>y)/;");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("invalid regular expression: Duplicate capture group name"),
        "{errors:?}"
    );
}

#[test]
fn promises_microtasks_and_timers_run_in_node_order() {
    // The expected order is Node 22's for the same source.
    let output = result(
        r#"
        const log = [];
        const out = (x) => log.push(String(x));
        setTimeout(() => {
          out('timeout0');
          setImmediate(() => out('immediate'));
          setTimeout(() => out('timeout-after'), 0);
          setTimeout(() => out('timeout5'), 5);
          const interval = setInterval(() => { out('interval'); clearInterval(interval); }, 3);
        }, 0);
        queueMicrotask(() => out('micro1'));
        Promise.resolve().then(() => out('then1')).then(() => out('then2'));
        const thenable = { then(resolve) { out('thenable.then'); resolve('t'); } };
        Promise.resolve(thenable).then((v) => out('thenable:' + v));
        new Promise((resolve, reject) => { out('executor'); reject(new Error('boom')); }).catch((e) => out('caught:' + e.message));
        Promise.all([1, Promise.resolve(2), { then(r) { r(3); } }]).then((v) => out('all:' + v));
        Promise.allSettled([1, Promise.reject(new Error('x'))]).then((r) => out('settled:' + r.map((e) => e.status)));
        Promise.any([Promise.reject(1), Promise.reject(2)]).catch((e) => out('any:' + e.constructor.name + ':' + e.errors + ':' + e.message));
        Promise.race([new Promise((r) => setTimeout(r, 2, 'slow')), 'fast']).then((v) => out('race:' + v));
        Promise.resolve(1).finally(() => out('finally')).then((v) => out('after finally:' + v));
        const { promise, resolve } = Promise.withResolvers();
        promise.then((v) => out('withResolvers:' + v));
        resolve(9);
        const cleared = setTimeout(() => out('never'), 3);
        clearTimeout(cleared);
        const handle = setTimeout(() => {}, 0);
        out(typeof handle + ':' + typeof handle[Symbol.toPrimitive] + ':' + typeof handle.ref + ':' + handle.hasRef());
        setTimeout(() => out('unref never'), 50).unref();
        out('sync end');
        export const result = { toString() { return log.join('|'); } };
    "#,
    );
    assert_eq!(
        output,
        "executor|object:function:function:true|sync end|micro1|then1|thenable.then|caught:boom|finally|withResolvers:9|then2|thenable:t|settled:fulfilled,rejected|any:AggregateError:1,2:All promises were rejected|race:fast|all:1,2,3|after finally:1|timeout0|immediate|timeout-after|interval|timeout5"
    );
}

#[test]
fn unhandled_rejections_end_the_event_loop() {
    let output = result(
        "Promise.reject(new Error('lost')); setTimeout(() => { throw 'never'; }, 1); export const result = 'done';",
    );
    assert_eq!(output, "event loop threw Error: lost\ndone");
}

#[test]
fn generators_and_async_functions_run_as_native_coroutines() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        r#"
const log = [];
const out = (x) => log.push(String(x));
function* counter(limit) {
  let received = 0;
  for (let i = 0; i < limit; i++) {
    received = yield i + received;
  }
  return 'end';
}

const c = counter(3);
out([c.next('ignored').value, c.next(10).value, c.next(20).value, JSON.stringify(c.next(30)), JSON.stringify(c.next())].join(','));
function* guarded() {
  try {
    yield 1;
    yield 2;
  } catch (e) {
    out('caught ' + e);
    yield 3;
  } finally {
    out('finally');
  }
}
const g = guarded();
out(g.next().value);
out(JSON.stringify(g.return('early')));
const t = guarded();
t.next();
out(JSON.stringify(t.throw('boom')));
out(JSON.stringify(t.next()));
function* inner() { const x = yield 'a'; yield 'b' + x; return 'inner done'; }
function* outer() { const r = yield* inner(); yield r; yield* [7, 8]; }
const o = outer();
out([o.next().value, o.next('!').value, o.next().value, o.next().value, o.next().value, o.next().done].join(','));
out([...counter(4)].join(','));
for (const value of guarded()) { out('loop ' + value); break; }
const obj = { *items() { yield 'x'; yield 'y'; } };
class Bag { constructor() { this.list = [1, 2]; } *[Symbol.iterator]() { yield* this.list; } }
out([...obj.items(), ...new Bag()].join(','));
out(Object.prototype.toString.call(counter(1)) + ' ' + (typeof counter(1)[Symbol.iterator]));
const sleep = (ms, v) => new Promise((resolve) => setTimeout(resolve, ms, v));
async function load(name) {
  out('load start ' + name);
  const value = await sleep(2, name.toUpperCase());
  out('loaded ' + value);
  return value + '!';
}
async function fail() { await null; throw new Error('async fail'); }
const arrow = async (x) => (await x) * 2;
class Service { async fetch(v) { return 'fetched ' + (await v); } }
(async () => {
  const results = await Promise.all([load('a'), arrow(21), new Service().fetch(Promise.resolve('data'))]);
  out(results.join('|'));
  try { await fail(); } catch (e) { out('caught ' + e.message); }
  out('await plain ' + (await 5));
})();
Promise.resolve().then(() => out('then before await resumes'));
out('sync end');
export const result = { toString() { return log.join('\n'); } };
"#,
    );
    assert_eq!(
        output,
        r#"0,11,22,{"value":"end","done":true},{"done":true}
1
finally
{"value":"early","done":true}
caught boom
{"value":3,"done":false}
finally
{"done":true}
a,b!,inner done,7,8,true
0,NaN,NaN,NaN
loop 1
finally
x,y,1,2
[object Generator] function
load start a
sync end
then before await resumes
loaded A
A!|42|fetched data
caught async fail
await plain 5"#
    );
}

#[test]
fn async_class_method_recaptures_its_home_before_returning_the_body() {
    let module = compile_dynamic(
        "class Base { get value() { return 1; } } class Derived extends Base { async read() { await null; return super.value; } } export const instance = new Derived();",
        Path::new("async-super.js"),
    )
    .unwrap();
    let source = module.rust_source();
    let async_factory = source.find("coroutine::async_function").unwrap();
    let function = &source[async_factory..];
    let body = function.find("Box::new(move |realm: &mut Realm|").unwrap();
    let factory = &function[..body];
    assert!(factory.contains("let home"), "{factory}");
    assert!(factory.contains("= home"), "{factory}");
}

#[test]
fn generator_returns_pass_through_catch_clauses() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        r#"
        const log = [];
        function* plain() { try { yield 1; } catch (e) { log.push('never'); } log.push('after'); }
        function* nested() { try { try { yield; } catch (e) { log.push('inner never'); } } finally { log.push('outer finally'); } }
        const p = plain(); p.next(); log.push(JSON.stringify(p.return('r')));
        const q = nested(); log.push(JSON.stringify(q.next())); log.push(JSON.stringify(q.return('s')));
        export const result = log.join('|');
    "#,
    );
    assert_eq!(
        output,
        r#"{"value":"r","done":true}|{"done":false}|outer finally|{"value":"s","done":true}"#
    );
}

#[test]
fn returns_and_jumps_run_finally_blocks_first() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        r#"
const log = [];
const out = (x) => log.push(String(x));
function simple() { try { return 'try'; } finally { out('finally 1'); } }
out(simple());
function nested() {
  try {
    try { return 'inner'; } finally { out('inner finally'); }
  } finally { out('outer finally'); }
}
out(nested());
function overriding() { try { return 'try'; } finally { return 'finally'; } }
out(overriding());
function fromCatch() { try { throw new Error('x'); } catch (e) { return 'catch ' + e.message; } finally { out('after catch'); } }
out(fromCatch());
function loops() {
  const seen = [];
  for (let i = 0; i < 5; i++) {
    try {
      if (i === 1) continue;
      if (i === 3) break;
      seen.push(i);
    } finally {
      seen.push('f' + i);
    }
  }
  return seen.join(',');
}
out(loops());
function labeled() {
  const seen = [];
  outer: for (const a of [1, 2]) {
    for (const b of [1, 2]) {
      try {
        if (b === 2) continue outer;
        seen.push(a + '' + b);
      } finally { seen.push('f'); }
    }
  }
  return seen.join(',');
}
out(labeled());
function closing() {
  const iterable = { [Symbol.iterator]() { let n = 0; return { next() { return { value: n++, done: false }; }, return() { out('closed'); return {}; } }; } };
  for (const value of iterable) {
    try { if (value === 1) return 'returned ' + value; } finally { out('loop finally ' + value); }
  }
}
out(closing());
async function asyncReturn() { try { await null; return 'async'; } finally { out('async finally'); } }
function* generatorReturn() { try { yield 1; return 'gen'; } finally { out('gen finally'); } }
const g = generatorReturn();
g.next();
out(JSON.stringify(g.next()));
asyncReturn().then((v) => out(v));
export const result = { toString() { return log.join('\n'); } };
"#,
    );
    assert_eq!(
        output,
        r#"finally 1
try
inner finally
outer finally
inner
finally
after catch
catch x
0,f0,f1,2,f2,f3
11,f,f,21,f,f
loop finally 0
loop finally 1
closed
returned 1
gen finally
{"value":"gen","done":true}
async finally
async"#
    );
}

#[test]
fn process_identity_reads_lower_to_native_target_values_and_live_pid() {
    let output = result(
        r#"
        function local(process) { process.platform += '!'; return process.platform + ':' + typeof process; }
        function primitive(process) { return typeof process; }
        function temporal() { const before = typeof process; let process = {}; return before; }
        const out = [process.platform, process['arch'], process?.pid > 0,
            Number.isInteger(process?.['pid']), process.pid === process['pid'],
            process['pla\u0074form'], process?.arch, typeof process, typeof process.platform,
            local({platform:'local'}), primitive(7), ({platform:'ordinary'}).platform];
        try { temporal(); } catch (error) { out.push(error.name); }
        let evaluations = 0;
        try { process.platform(++evaluations); } catch (error) { out.push(error.name, evaluations); }
        try { process['pid'](); } catch (error) { out.push(error.name); }
        export const result = out.join('|');
    "#,
    );
    assert_eq!(
        output,
        "win32|x64|true|true|true|win32|x64|object|string|local!:object|number|ordinary|ReferenceError|TypeError|1|TypeError"
    );
    let diagnosed = errors("process.stderr['cursorTo'];").join("\n");
    assert!(diagnosed.contains("blocked.ts:1:"), "{diagnosed}");
    assert!(diagnosed.contains("is not compiled yet"), "{diagnosed}");
    let output = result(
        "const start = process.hrtime(); const diff = process.hrtime(start); export const result = [Array.isArray(start), start.length, start[0] >= 0, start[1] >= 0 && start[1] < 1e9, Array.isArray(diff), typeof process.hrtime.bigint()].join('|');",
    );
    assert_eq!(output, "true|2|true|true|true|bigint");
    // The process object is ordinary: aliases, dynamic keys and mutations
    // compile.
    let output = result(
        r#"
        const alias = process; const key = 'platform'; let failed = '';
        try { process.platform = 'other'; } catch (error) { failed = error.name; }
        process.custom = 1; process.custom++;
        delete process.pid; process.arch ??= 'unused';
        const stream = process.stdout;
        export const result = [alias[key], failed, process.custom, 'pid' in process, process.arch,
            process.stdout.isTTY, typeof stream.write].join('|');
    "#,
    );
    assert_eq!(output, "win32|TypeError|2|false|x64||function");
}

#[test]
fn process_stream_error_listeners_link_the_native_event_emitter() {
    let module = compile_dynamic(
        "process.stdout.on('error', () => {}); process.stderr.on('error', () => {});",
        Path::new("stream-listener.ts"),
    )
    .unwrap();
    assert!(module.native_dependencies().contains("node:events"));
}

#[test]
fn process_path_reads_and_function_aliases_lower_to_native_operations() {
    let output = result(
        r#"
        const cwd = process.cwd;
        let evaluations = 0;
        const path = cwd.call(null, ++evaluations);
        const out = [typeof cwd, cwd.name, cwd.length, cwd === process['cwd'],
            path === process?.cwd(), path === process['cwd']({toString(){throw 'unused';}}),
            path === cwd.apply(null, [42]), path === process.cwd?.(),
            typeof process.execPath, process.execPath === process['execPath'],
            process?.['execPath'].endsWith('.exe'), evaluations];
        out.push(typeof new process.cwd());
        try { process.execPath(++evaluations); } catch (error) { out.push(error.name, evaluations); }
        function local(process) { return process.cwd() + process.execPath; }
        out.push(local({cwd(){return 'local';},execPath:'!'}));
        out.push(process.binding('buffer').kStringMaxLength);
        export const result = out.join('|');
    "#,
    );
    assert_eq!(
        output,
        "function|wrappedCwd|0|true|true|true|true|true|string|true|true|1|object|TypeError|2|local!|536870888"
    );
    assert!(compile_dynamic("process.chdir('C:/');", Path::new("supported.ts")).is_ok());
}

#[test]
fn windows_environment_aliases_mutations_and_snapshots_execute_natively() {
    let output = result(
        r#"
        const env = process['env'];
        const key = 'NATIVEBOAT_COMPILED_ENV_' + process.pid;
        const out = [env === process.env, typeof env];
        delete env[key];
        out.push(env[key] === undefined, !(key in env));
        env[key] = '';
        out.push(env[key] === '', key in env);
        env[key] = 42;
        out.push(env[key.toLowerCase()], env[key]++, env[key]);
        env[key] = null; out.push(env[key] === 'null');
        env[key] = undefined; out.push(env[key] === 'undefined');
        let conversions = 0;
        env[key] = {toString(){conversions++;return '中🛤';}};
        const snapshot = {...env};
        env[key] = 'changed';
        out.push(snapshot[key], conversions, Object.keys(env).includes(key));
        env[key] = '\ud800\0tail'; out.push(env[key] === '�');
        env[key + '\0suffix'] = 'truncated'; out.push(env[key]);
        let keyConversions = 0;
        const dynamicKey = {[Symbol.toPrimitive](hint){keyConversions++;return key;}};
        out.push(env[dynamicKey], keyConversions);
        const symbol = Symbol('env');
        let valueConversions = 0;
        try { env[symbol] = {toString(){valueConversions++;return 'unused';}}; } catch(error) { out.push(error.name, valueConversions); }
        try { env[key] = symbol; } catch(error) { out.push(error.name); }
        try { env[key] = {toString(){throw 'conversion';}}; } catch(error) { out.push(error); }
        out.push(env[symbol] === undefined, delete env[symbol]);
        const prototype = Object.getPrototypeOf(env); prototype[symbol] = 7;
        const inherited = Object.create(env); inherited[key] = 'own';
        out.push(inherited[symbol] === undefined, symbol in inherited, inherited[key], env[key]);
        delete prototype[symbol];
        Object.defineProperty(env, key, {value:7,writable:true,enumerable:true,configurable:true});
        const descriptor = Object.getOwnPropertyDescriptor(env,key);
        out.push(descriptor.value, descriptor.writable, descriptor.enumerable, descriptor.configurable);
        for (const descriptor of [{value:1}, {get(){return 1;}}]) {
            try { Object.defineProperty(env,key,descriptor); } catch(error) { out.push(error.name,error.code); }
        }
        for (const operation of ['freeze','seal','preventExtensions']) {
            try { Object[operation](env); } catch(error) { out.push(error.name,Object.isExtensible(env)); }
        }
        Object.assign(env, {[key]:'assigned'});
        out.push(env[key], Object.hasOwn(env,key), delete env[key], env[key] === undefined);
        function local(process) { return process.env.value; }
        out.push(local({env:{value:'local'}}));
        export const result = out.join('|');
    "#,
    );
    assert_eq!(
        output,
        "true|object|true|true|true|true|42|42|43|true|true|中🛤|1|true|true|truncated|truncated|1|TypeError|0|TypeError|conversion|true|true|true|true|own|truncated|7|true|true|true|TypeError|ERR_INVALID_OBJECT_DEFINE_PROPERTY|TypeError|ERR_INVALID_OBJECT_DEFINE_PROPERTY|TypeError|true|TypeError|true|TypeError|true|assigned|true|true|true|local"
    );
    let diagnosed = errors("import {env} from 'node:process'; env;").join("\n");
    assert!(diagnosed.contains("blocked.ts:1:"), "{diagnosed}");
}

#[test]
fn packaged_native_arguments_preserve_array_identity_and_mutation() {
    let output = result(
        r#"
        const argv = process['argv'];
        const original = argv[0];
        const length = argv.length;
        const out = [Array.isArray(argv), argv === process.argv, argv === process?.['argv'],
            argv[0] === process.execPath, length >= 1];
        argv.push('追加'); out.push(argv.length === length + 1, process.argv[length]);
        const copy = argv.slice(); copy[0] = 'copy'; out.push(argv[0] === original);
        argv[0] = 'alias'; out.push(process.argv[0]);
        let evaluations = 0;
        try { process.argv(++evaluations); } catch(error) { out.push(error.name,evaluations); }
        function local(process) { return process.argv[0]; }
        out.push(local({argv:['local']}));
        export const result = out.join('|');
    "#,
    );
    assert_eq!(
        output,
        "true|true|true|true|true|true|追加|true|alias|TypeError|1|local"
    );
    let diagnosed = errors("import {argv} from 'node:process'; argv;").join("\n");
    assert!(diagnosed.contains("blocked.ts:1:"), "{diagnosed}");
}

#[test]
fn object_literal_proto_entries_set_the_prototype() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        r#"
        const base = { greet() { return 'hi'; } };
        const a = { __proto__: base, x: 1 };
        const b = { '__proto__': null };
        const c = { __proto__: 5, y: 2 };
        const __proto__ = { z: 3 };
        const d = { __proto__ };
        const e = { ['__proto__']: 4 };
        const f = { __proto__() { return 'm'; } };
        const log = [a.greet(), Object.getPrototypeOf(b) === null, Object.getPrototypeOf(c) === Object.prototype, Object.keys(c).join(), Object.keys(d).join(), Object.getPrototypeOf(d) === Object.prototype, Object.keys(e).join(), typeof Object.getOwnPropertyDescriptor(f, '__proto__').value, Object.keys(a).join()];
        export const result = log.join('|');
    "#,
    );
    assert_eq!(output, "hi|true|true|y|__proto__|true|__proto__|function|x");
}

#[test]
fn reflect_exposes_internal_methods() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        r#"
        const out = [];
        out.push(Reflect.apply(Math.max, null, [1, 5, 3]));
        class Base { constructor(x) { this.x = x; } }
        class Other {}
        const made = Reflect.construct(Base, [7], Other);
        out.push(made.x, Object.getPrototypeOf(made) === Other.prototype);
        const frozen = Object.freeze({ a: 1 });
        out.push(Reflect.defineProperty(frozen, 'b', { value: 2 }), Reflect.defineProperty({}, 'c', { value: 3 }));
        out.push(Reflect.deleteProperty(frozen, 'a'), Reflect.deleteProperty({ d: 1 }, 'd'));
        const source = { get who() { return this.name; }, set who(v) { this.seen = v; } };
        const receiver = { name: 'receiver' };
        out.push(Reflect.get(source, 'who', receiver), Reflect.get(source, 'missing'));
        out.push(Reflect.set(source, 'who', 'value', receiver), receiver.seen);
        out.push(Reflect.set(frozen, 'a', 9), Reflect.set({}, 'e', 1));
        const keyed = { b: 1, 2: 'two', a: 2, [Symbol.iterator]: 0 };
        out.push(Reflect.ownKeys(keyed).map(String).join(','));
        out.push(Reflect.has(keyed, 'toString'), Reflect.isExtensible(frozen));
        const sealed = {}; out.push(Reflect.preventExtensions(sealed), Reflect.setPrototypeOf(sealed, null), Reflect.setPrototypeOf({}, null));
        out.push(JSON.stringify(Reflect.getOwnPropertyDescriptor(keyed, 'a')), Reflect.getOwnPropertyDescriptor(keyed, 'zz'), Reflect.getPrototypeOf([]) === Array.prototype);
        const re = /x/; Reflect.defineProperty(re, 'test', { value: () => 'fast' }); out.push(re.test());
        out.push(Object.prototype.toString.call(Reflect));
        try { Reflect.get(1, 'a'); } catch (e) { out.push(e.message); }
        export const result = out.join('|');
    "#,
    );
    assert_eq!(
        output,
        r#"5|7|true|false|true|false|true|receiver||true|value|false|true|2,b,a,Symbol(Symbol.iterator)|true|false|true|false|true|{"value":2,"writable":true,"enumerable":true,"configurable":true}||true|fast|[object Reflect]|Reflect.get called on non-object"#
    );
}

#[test]
fn uri_functions_and_global_this_are_native_globals() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        "globalThis.marker = 1; export const result = [global.marker, typeof globalThis, decodeURIComponent(encodeURIComponent('é/😀')), unescape(escape('ü')), encodeURI('a b')].join('|');",
    );
    assert_eq!(output, "1|object|é/😀|ü|a%20b");
}

#[test]
fn dates_compile_to_native_time_values() {
    // The expected text is Node 22's output; it does not depend on the zone.
    let output = result(
        r#"
        const d = new Date(Date.UTC(2026, 9, 11, 7, 30, 5, 123));
        const local = new Date(2026, 0, 1);
        const out = [d.toISOString(), d.toUTCString(), JSON.stringify({ d }), d.getUTCDay(), Date.UTC(2000, 1, 29), local.getTime() === Date.UTC(2026, 0, 1) + local.getTimezoneOffset() * 60000, Date.parse('2026-10-11T07:30:05Z'), new Date(0) < new Date(1), +new Date(5), typeof Date.now(), String(new Date(NaN)), new Date(d).getTime() === d.getTime()];
        d.setUTCFullYear(2030); out.push(d.toISOString());
        export const result = out.join('|');
    "#,
    );
    assert_eq!(
        output,
        r#"2026-10-11T07:30:05.123Z|Sun, 11 Oct 2026 07:30:05 GMT|{"d":"2026-10-11T07:30:05.123Z"}|0|951782400000|true|1791703805000|true|5|number|Invalid Date|true|2030-10-11T07:30:05.123Z"#
    );
}

#[test]
fn typed_arrays_buffers_and_views_compile() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        r#"
const out = [];
const t = (f) => { try { return String(f()); } catch (e) { return e.constructor.name + ': ' + e.message; } };
const u8 = new Uint8Array([1, 2, 300, -1]);
out.push(u8.join(), u8.length, u8.byteLength, Uint8Array.BYTES_PER_ELEMENT, u8.BYTES_PER_ELEMENT);
const clamped = new Uint8ClampedArray([1.5, 2.5, -3, 300, NaN]);
out.push(clamped.join());
const i16 = Int16Array.from([40000, -40000, 1.9]);
out.push(i16.join(), Int16Array.of(1, 2).join());
const f32 = new Float32Array([0.1, 1e40]);
out.push(f32[0], f32[1]);
const buffer = new ArrayBuffer(8);
const view = new DataView(buffer);
view.setUint16(0, 0x1234);
view.setUint16(2, 0x1234, true);
view.setFloat32(4, 1.5);
const bytes = new Uint8Array(buffer);
out.push(bytes.join(), view.getUint16(0), view.getUint16(0, true), view.getFloat32(4), view.getInt8(0), view.byteLength);
const window = new Uint16Array(buffer, 2, 1);
out.push(window[0], window.byteOffset, window.buffer === buffer);
const sub = bytes.subarray(1, 3);
sub[0] = 99;
out.push(bytes[1], sub.length, bytes.slice(-2).join());
out.push(new Int32Array(3).fill(7, 1).join(), [...new Uint8Array([3, 1, 2]).sort()].join(), new Float64Array([3, NaN, -0, 0, -1]).sort().join());
out.push(new Uint8Array([1, 2, 3]).map((x) => x * 100).join(), new Uint8Array([1, 2, 3, 4]).filter((x) => x % 2).join());
out.push(new Uint8Array([5, 6]).indexOf(6), new Uint8Array([5, 6]).includes(5), new Uint8Array([1, 2, 3]).reduce((a, b) => a + b));
const target = new Uint8Array(4); target.set([9, 8], 1); target.set(new Uint8Array([7]), 3);
out.push(target.join(), t(() => target.set([1, 2, 3], 2)));
out.push(new Uint8Array([1, 2, 3]).toReversed().join(), new Uint8Array([3, 1]).toSorted().join(), new Uint8Array([1, 2]).with(-1, 9).join());
out.push(Object.prototype.toString.call(u8), u8[Symbol.toStringTag], ArrayBuffer.isView(view), ArrayBuffer.isView(buffer));
out.push(t(() => new Uint8Array(-1)), t(() => new Uint16Array(new ArrayBuffer(3))), t(() => new DataView({})), t(() => view.getUint32(6)));
out.push(u8[10], u8['-0'], '1.5' in u8, Object.keys(new Uint8Array(2)).join(), JSON.stringify(new Uint8Array([1, 2])));
const keyed = new Uint8Array(1); keyed.extra = 'x'; keyed[5] = 1; out.push(Object.keys(keyed).join(), keyed[5]);
out.push(buffer.slice(4).byteLength, new Uint8Array(buffer.slice(0, 2)).join());
export const result = out.join('|');
"#,
    );
    assert_eq!(
        output,
        r#"1,2,44,255|4|4|1|1|2,2,0,255,0|-25536,25536,1|1,2|0.10000000149011612|Infinity|18,52,52,18,63,192,0,0|4660|13330|1.5|18|8|4660|2|true|99|2|0,0|0,7,7|1,2,3|-1,0,0,3,NaN|100,200,44|1,3|1|true|6|0,9,8,7|RangeError: offset is out of bounds|3,2,1|1,3|1,9|[object Uint8Array]|Uint8Array|true|false|RangeError: Invalid typed array length: -1|RangeError: byte length of Uint16Array should be a multiple of 2|TypeError: First argument to DataView constructor must be an ArrayBuffer|RangeError: Offset is outside the bounds of the DataView|||false|0,1|{"0":1,"1":2}|0,extra||4|18,99"#
    );
}

#[test]
fn native_process_object_matches_node() {
    // The expected text is Node 22's output for the same source.
    let output = result(
        r#"
export let result = '';
const out = [];
const t = (f) => { try { return String(f()); } catch (e) { return `${e.name}:${e.code}:${e.message}:${String(e)}:${Object.keys(e)}`; } };
const global = Object.getOwnPropertyDescriptor(globalThis, 'process');
out.push(typeof global.get, typeof global.set, global.enumerable, global.configurable);
out.push(process === globalThis.process, typeof process, Object.prototype.toString.call(process));
out.push(process.version === 'v' + process.versions.node, process.arch, process.platform);
for (const key of ['version', 'versions', 'arch', 'platform', 'exitCode', 'exit', 'nextTick', 'emitWarning', 'stdout', 'cwd', 'env', 'argv', 'execArgv', 'pid', 'execPath', 'argv0']) {
  const d = Object.getOwnPropertyDescriptor(process, key);
  out.push(key + ('value' in d ? ':data:' + d.writable : ':accessor') + ':' + d.enumerable + ':' + d.configurable);
}
const v = Object.getOwnPropertyDescriptor(process.versions, 'node');
out.push(v.writable, v.enumerable, v.configurable);
out.push(process.exit.name, process.exit.length, process.nextTick.length, process.emitWarning.length, process.cwd.name);
out.push(process.execArgv.length, Array.isArray(process.execArgv), process.pid > 0, process.execPath.endsWith('.exe'));
out.push(process.argv[0] === process.execPath, typeof process.argv0, process.env === process.env, process.cwd() === process.cwd.call(1));
out.push(t(() => process.exitCode), t(() => { process.exitCode = '0x10'; return process.exitCode; }), typeof process.exitCode);
out.push(t(() => { process.exitCode = null; return process.exitCode; }));
for (const value of ['abc', 1.5, 2 ** 53, -(2 ** 53), 1e21, '', "a'b", 'x'.repeat(30), true, Symbol('s'), {}, Object.create(null), Object.assign(Object.create(null), {a: 1}), [], [1], function foo() {}, new (class Foo {})(), {constructor: {}}, {constructor: {}, a: 1}, NaN, Infinity, -0, 5e9 + 0.5]) {
  out.push(t(() => { process.exitCode = value; return process.exitCode; }));
}
process.exitCode = undefined;
out.push(t(() => process.nextTick(1)), t(() => process.nextTick()));
out.push(t(() => process.emitWarning(1)), t(() => process.emitWarning('x', 5)), t(() => process.emitWarning('x', 'T', 5)), t(() => process.emitWarning('x', [])));
out.push(t(() => process.emitWarning('x', {type: 5})));
out.push(t(() => process.stdout.write(1)), t(() => process.stdout.write(null)), t(() => process.stdout.write('x', 'nope')), t(() => process.stdout.write('x', 'buffer')), t(() => process.stderr.write({}, 'utf8')), t(() => process.stdout.write('x', Symbol('e'))));
out.push(process.stdout.fd, process.stderr.fd, process.stdout === process.stdout, process.stdout.write.length, Object.keys(process.stdout).includes('write'));
const order = [];
Promise.resolve().then(() => { order.push('p1'); process.nextTick(() => order.push('t3')); Promise.resolve().then(() => order.push('p2')); });
process.nextTick((a, b) => { order.push('t1' + a + b); process.nextTick(() => order.push('t2')); }, 'x', 'y');
process.stdout.write('', (...args) => order.push('cb' + args.length + args[0]));
process.stdout.write('', 'utf8', 'not a function');
process.stdout.write(new Uint8Array(0), () => order.push('typed'));
queueMicrotask(() => order.push('m'));
setTimeout(() => {
  out.push(order.join());
  const deprecation = new Error('dep'); deprecation.name = 'DeprecationWarning';
  process.noDeprecation = true; process.emitWarning(deprecation); process.noDeprecation = false;
  globalThis.process = 5;
  out.push(globalThis.process, Object.getOwnPropertyDescriptor(globalThis, 'process').enumerable);
  result = out.join('|');
}, 0);
"#,
    );
    assert_eq!(
        output,
        r#"function|function|false|true|true|object|[object process]|true|x64|win32|version:data:false:true:true|versions:data:false:true:true|arch:data:false:true:true|platform:data:false:true:true|exitCode:accessor:true:false|exit:data:true:true:true|nextTick:data:true:true:true|emitWarning:data:true:true:true|stdout:accessor:true:true|cwd:data:true:true:true|env:data:true:true:true|argv:data:true:true:true|execArgv:data:true:true:true|pid:data:false:true:true|execPath:data:true:true:true|argv0:data:false:true:false|false|true|true|exit|1|1|4|wrappedCwd|0|true|true|true|true|string|true|true|undefined|16|number|undefined|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received type string ('abc'):TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received type string ('abc'):code|RangeError:ERR_OUT_OF_RANGE:The value of "code" is out of range. It must be an integer. Received 1.5:RangeError [ERR_OUT_OF_RANGE]: The value of "code" is out of range. It must be an integer. Received 1.5:code|RangeError:ERR_OUT_OF_RANGE:The value of "code" is out of range. It must be >= -9007199254740991 && <= 9007199254740991. Received 9_007_199_254_740_992:RangeError [ERR_OUT_OF_RANGE]: The value of "code" is out of range. It must be >= -9007199254740991 && <= 9007199254740991. Received 9_007_199_254_740_992:code|RangeError:ERR_OUT_OF_RANGE:The value of "code" is out of range. It must be >= -9007199254740991 && <= 9007199254740991. Received -9_007_199_254_740_992:RangeError [ERR_OUT_OF_RANGE]: The value of "code" is out of range. It must be >= -9007199254740991 && <= 9007199254740991. Received -9_007_199_254_740_992:code|RangeError:ERR_OUT_OF_RANGE:The value of "code" is out of range. It must be >= -9007199254740991 && <= 9007199254740991. Received 1e_+21:RangeError [ERR_OUT_OF_RANGE]: The value of "code" is out of range. It must be >= -9007199254740991 && <= 9007199254740991. Received 1e_+21:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received type string (''):TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received type string (''):code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received type string ("a'b"):TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received type string ("a'b"):code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received type string ('xxxxxxxxxxxxxxxxxxxxxxxxx...'):TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received type string ('xxxxxxxxxxxxxxxxxxxxxxxxx...'):code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received type boolean (true):TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received type boolean (true):code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received type symbol (Symbol(s)):TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received type symbol (Symbol(s)):code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received an instance of Object:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received an instance of Object:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received [Object: null prototype] {}:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received [Object: null prototype] {}:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received [Object: null prototype]:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received [Object: null prototype]:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received an instance of Array:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received an instance of Array:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received an instance of Array:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received an instance of Array:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received function foo:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received function foo:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received an instance of Foo:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received an instance of Foo:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received [Object]:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received [Object]:code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type number. Received [Object]:TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type number. Received [Object]:code|RangeError:ERR_OUT_OF_RANGE:The value of "code" is out of range. It must be an integer. Received NaN:RangeError [ERR_OUT_OF_RANGE]: The value of "code" is out of range. It must be an integer. Received NaN:code|RangeError:ERR_OUT_OF_RANGE:The value of "code" is out of range. It must be an integer. Received Infinity:RangeError [ERR_OUT_OF_RANGE]: The value of "code" is out of range. It must be an integer. Received Infinity:code|0|RangeError:ERR_OUT_OF_RANGE:The value of "code" is out of range. It must be an integer. Received 5000000000.5:RangeError [ERR_OUT_OF_RANGE]: The value of "code" is out of range. It must be an integer. Received 5000000000.5:code|TypeError:ERR_INVALID_ARG_TYPE:The "callback" argument must be of type function. Received type number (1):TypeError [ERR_INVALID_ARG_TYPE]: The "callback" argument must be of type function. Received type number (1):code|TypeError:ERR_INVALID_ARG_TYPE:The "callback" argument must be of type function. Received undefined:TypeError [ERR_INVALID_ARG_TYPE]: The "callback" argument must be of type function. Received undefined:code|TypeError:ERR_INVALID_ARG_TYPE:The "warning" argument must be of type string or an instance of Error. Received type number (1):TypeError [ERR_INVALID_ARG_TYPE]: The "warning" argument must be of type string or an instance of Error. Received type number (1):code|TypeError:ERR_INVALID_ARG_TYPE:The "type" argument must be of type string. Received type number (5):TypeError [ERR_INVALID_ARG_TYPE]: The "type" argument must be of type string. Received type number (5):code|TypeError:ERR_INVALID_ARG_TYPE:The "code" argument must be of type string. Received type number (5):TypeError [ERR_INVALID_ARG_TYPE]: The "code" argument must be of type string. Received type number (5):code|TypeError:ERR_INVALID_ARG_TYPE:The "type" argument must be of type string. Received an instance of Array:TypeError [ERR_INVALID_ARG_TYPE]: The "type" argument must be of type string. Received an instance of Array:code|TypeError:ERR_INVALID_ARG_TYPE:The "type" argument must be of type string. Received type number (5):TypeError [ERR_INVALID_ARG_TYPE]: The "type" argument must be of type string. Received type number (5):code|TypeError:ERR_INVALID_ARG_TYPE:The "chunk" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received type number (1):TypeError [ERR_INVALID_ARG_TYPE]: The "chunk" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received type number (1):code|TypeError:ERR_STREAM_NULL_VALUES:May not write null values to stream:TypeError [ERR_STREAM_NULL_VALUES]: May not write null values to stream:code|TypeError:ERR_UNKNOWN_ENCODING:Unknown encoding: nope:TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: nope:code|TypeError:ERR_UNKNOWN_ENCODING:Unknown encoding: buffer:TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: buffer:code|TypeError:ERR_INVALID_ARG_TYPE:The "chunk" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received an instance of Object:TypeError [ERR_INVALID_ARG_TYPE]: The "chunk" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received an instance of Object:code|TypeError:ERR_UNKNOWN_ENCODING:Unknown encoding: Symbol(e):TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: Symbol(e):code|1|2|true|3|false|t1xy,cb1null,typed,t2,p1,m,p2,t3|5|false"#
    );
}

#[test]
fn native_modules_link_runtime_exports_for_imports_and_requires() {
    let module = mixed("main.ts", &[
        ("main.ts", "import errors from 'errors.native'; import required from 'dep.cjs'; const error = new errors.codes.ERR_INVALID_ARG_TYPE('a', 'string', 1); export const result = [errors === required, error.code, error instanceof TypeError].join('|');"),
        ("dep.cjs", "'use strict'; module.exports = require('errors.native');"),
        ("errors.native", "internal/errors"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); let text = show(&mut realm, &value); println!(\"{text}\");",
        ),
        "true|ERR_INVALID_ARG_TYPE|true"
    );
}

#[test]
fn es_imports_take_commonjs_named_exports_and_namespaces_like_node() {
    // The expected text is Node 22's output for the same modules.
    let module = mixed("main.ts", &[
        ("main.ts", "import def, { a, b, later } from 'dep.cjs'; import * as ns from 'dep.cjs'; import { reZ, reNs } from 'reexport.ts'; const out = [def === ns.default, a, b(), later, Object.keys(ns).join(), Object.prototype.toString.call(ns), Object.isExtensible(ns), reZ, reNs === ns]; export let result = ''; setTimeout(() => { result = [...out, later, def.later, ns.later].join('|'); }, 5);"),
        ("reexport.ts", "export { z as reZ } from 'dep.cjs'; export * as reNs from 'dep.cjs';"),
        ("dep.cjs", "'use strict'; exports.a = 1; exports.b = function b() { return 'b'; }; exports.later = 'initial'; setTimeout(() => { exports.later = 'changed'; }, 0); module.exports.z = 'z';"),
    ]).unwrap();
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); let text = show(&mut realm, &value); println!(\"{text}\");",
        ),
        "true|1|b|initial|a,b,default,later,z|[object Module]|false|z|true|initial|changed|initial"
    );
}

#[test]
fn named_function_expressions_refer_to_themselves() {
    // The expected text is Node 22's output for the same module.
    let output = result(
        r#"
export let result = '';
const out = [];
const fact = function f(n) { return n <= 1 ? 1 : n * f(n - 1); };
out.push(fact(5), fact.name);
const counter = function count() { count.calls = (count.calls || 0) + 1; return count.calls; };
counter(); out.push(counter(), counter.calls);
const nested = function outer() { return [1, 2].map(() => typeof outer).join(); };
out.push(nested());
const shadowed = function g() { const g = 3; return g; };
out.push(shadowed());
const assign = function h() { try { h = 1; } catch (e) { return e.name + ':' + e.message + ':' + typeof h; } };
out.push(assign());
const gen = function* walk(n) { if (n > 0) { yield n; yield* walk(n - 1); } };
out.push([...gen(3)].join());
const later = async function wait(n) { return n > 0 ? await wait(n - 1) + 1 : 0; };
const f2 = function same() { return same; }; out.push(f2() === f2);
later(3).then((v) => { out.push(v); result = out.join('|'); });
"#,
    );
    assert_eq!(
        output,
        "120|f|2|2|function,function|3|TypeError:Assignment to constant variable.:function|3,2,1|true|3"
    );
}

#[test]
fn bigints_compute_compare_and_convert_like_node() {
    // The expected text is Node 22's output for the same module.
    let output = result(
        r#"
const out = [];
const show = (v) => typeof v === 'bigint' ? v + 'n' : Array.isArray(v) ? '[' + v.map(show).join(',') + ']' : JSON.stringify(v);
const t = (f) => { try { return show(f()); } catch (e) { return e.name + ': ' + e.message; } };
const cases = [
  () => 1n + 1, () => +1n, () => 1n / 0n, () => 2n ** -1n, () => 1n >>> 0n,
  () => BigInt(1.5), () => BigInt('x'), () => BigInt(undefined), () => BigInt(null), () => BigInt(Symbol()),
  () => new BigInt(1), () => JSON.stringify(1n), () => BigInt.prototype.toString.call(1), () => (1n).toString(37),
  () => BigInt('-0x10'), () => BigInt(' 0x1F '), () => BigInt(''), () => BigInt('1e3'), () => BigInt(1e21), () => BigInt(-0),
  () => BigInt(NaN), () => BigInt(Infinity), () => Math.abs(1n), () => 1n < 1.5, () => 2n > '1', () => 1n == '1', () => 1n == 1,
  () => 0n == false, () => -5n >> 1n, () => -5n / 2n, () => -5n % 2n, () => ~5n, () => -6n & 3n, () => -6n | 3n, () => -6n ^ 3n,
  () => (2n ** 100n).toString(16), () => Number(2n ** 64n + 1n), () => Number(-(2n ** 1100n)), () => BigInt.asIntN(8, 255n),
  () => BigInt.asUintN(8, -1n), () => BigInt.asIntN(-1, 1n), () => typeof Object(1n), () => Object.prototype.toString.call(1n),
  () => [1n, 2n].includes(1n), () => new Set([1n, 1n]).size, () => 'a' + 1n, () => `${10n}`, () => (1234567n).toLocaleString(),
  () => BigInt(true), () => 1n < 'x', () => parseInt('12n'), () => isNaN(1n), () => Number.isInteger(1n),
  () => [3n, 1n, 2n].sort().join(), () => 1n << -1n, () => (-7n) ** 3n, () => 0n / 1n, () => -0n, () => Object.is(0n, -0n),
  () => 10n ** 1000n > 0n, () => 2n ** (2n ** 40n),
  () => { let x = 5n; x++; ++x; x--; return [x, x++, x, -x, ~x]; },
  () => { let n = 1n; for (let i = 0; i < 70; i++) n *= 3n; return [n, n % 1000000007n, n / 3n ** 60n, n.toString(36)]; },
  () => (2n ** 128n - 1n) / (2n ** 64n + 3n), () => (-(2n ** 200n)) >> 100n, () => (-(2n ** 200n) - 1n) >> 100n,
  () => (123456789012345678901234567890n * 987654321098765432109876543210n) % 1000000000000000000007n,
  () => BigInt.asUintN(64, -(2n ** 63n)), () => BigInt.asIntN(64, 2n ** 64n - 1n), () => BigInt.asIntN(0, 5n),
  () => (2n ** 64n) - 1n == 18446744073709551615, () => 2n ** 53n + 1n > 2 ** 53, () => 9007199254740993n == 9007199254740992,
  () => [0n ? 1 : 2, !0n, !!1n, 0n || 'z'], () => Object(2n) + 1n, () => [1n].map((v) => v * 2n), () => String(-123n),
  () => BigInt('0b1010'), () => BigInt('0o777'), () => BigInt('+42'), () => BigInt('  -42  '), () => BigInt(Number.MAX_SAFE_INTEGER) + 2n,
  () => Number(123n), () => (255n).toString(2), () => (-255n).toString(16), () => BigInt.prototype.valueOf.call(Object(3n)),
  () => typeof 1n, () => 5n > 4, () => 4 >= 4n, () => 3n <= 2.5, () => -1n < -0.5, () => NaN < 1n,
];
for (const f of cases) out.push(t(f));
export const result = out.join('|');
"#,
    );
    assert_eq!(
        output,
        r#"TypeError: Cannot mix BigInt and other types, use explicit conversions|TypeError: Cannot convert a BigInt value to a number|RangeError: Division by zero|RangeError: undefined must be positive|TypeError: BigInts have no unsigned right shift, use >> instead|RangeError: The number 1.5 cannot be converted to a BigInt because it is not an integer|SyntaxError: Cannot convert x to a BigInt|TypeError: Cannot convert undefined to a BigInt|TypeError: Cannot convert null to a BigInt|TypeError: Cannot convert Symbol() to a BigInt|TypeError: BigInt is not a constructor|TypeError: Do not know how to serialize a BigInt|TypeError: BigInt.prototype.toString requires that 'this' be a BigInt|RangeError: toString() radix argument must be between 2 and 36|SyntaxError: Cannot convert -0x10 to a BigInt|31n|0n|SyntaxError: Cannot convert 1e3 to a BigInt|1000000000000000000000n|0n|RangeError: The number NaN cannot be converted to a BigInt because it is not an integer|RangeError: The number Infinity cannot be converted to a BigInt because it is not an integer|TypeError: Cannot convert a BigInt value to a number|true|true|true|true|true|-3n|-2n|-1n|-6n|2n|-5n|-7n|"10000000000000000000000000"|18446744073709552000|null|-1n|255n|RangeError: Invalid value: not (convertible to) a safe integer|"object"|"[object BigInt]"|true|1|"a1"|"10"|"1,234,567"|1n|false|12|TypeError: Cannot convert a BigInt value to a number|false|"1,2,3"|0n|-343n|0n|0n|true|true|RangeError: Maximum BigInt size exceeded|[6n,6n,7n,-7n,-8n]|[2503155504993241601315571986085849n,662963356n,59049n,"5798xmwn4ruxv92m5f80k9"]|18446744073709551613n|-1267650600228229401496703205376n|-1267650600228229401496703205377n|660143270901617131545n|9223372036854775808n|-1n|0n|false|true|false|[2,true,true,"z"]|3n|[2n]|"-123"|10n|511n|42n|-42n|9007199254740993n|123|"11111111"|"-ff"|3n|"bigint"|true|true|false|true|false"#
    );
}

#[test]
fn library_globals_read_through_the_node_library() {
    let module = compile_dynamic(
        "export const kinds = [typeof Buffer, typeof atob, typeof btoa];",
        Path::new("globals.ts"),
    )
    .unwrap();
    assert!(
        module
            .rust_source()
            .contains("::nativeboat_native::modules::global(realm, \"node:buffer\", \"Buffer\")")
    );
}

#[test]
fn web_crypto_global_reads_the_native_realm_binding() {
    let module = compile_dynamic(
        "export const bufferId = crypto.randomUUID();",
        Path::new("crypto-global.ts"),
    )
    .unwrap();
    let rust = module.rust_source();
    assert!(
        rust.contains("ops::get_static_at(realm, &global, &S")
            && rust.contains("StaticStr::new(\"crypto\")"),
        "{rust}"
    );
    assert!(!rust.contains("global 'crypto' is not compiled yet"));
}

#[test]
fn process_links_the_node_library_only_when_its_emitter_may_be_reached() {
    let linked = |source: &str| {
        compile_dynamic(source, Path::new("process.ts"))
            .unwrap()
            .rust_source()
            .contains("modules::global(realm, \"node:events\", \"process\")")
    };
    for source in [
        "export const a = [process.platform, process['env'], typeof process];",
        "process.exitCode = 1; delete process.title2; process.nextTick(() => {});",
    ] {
        assert!(!linked(source), "{source}");
    }
    for source in [
        "process.on('exit', () => {});",
        "process['once']('exit', () => {});",
        "export const p = process;",
        "const key = 'on'; process[key]('exit', () => {});",
        "process.on = null;",
        "delete process.emit;",
    ] {
        assert!(linked(source), "{source}");
    }
}

#[test]
fn async_delegation_matches_node_iterator_protocol_and_request_order() {
    assert_eq!(
        result(include_str!("tests/async_delegation.mjs")),
        include_str!("tests/async_delegation.expected.txt").trim_end()
    );
}

#[test]
fn async_generators_queue_requests_like_node() {
    // The expected text is Node 22's output for the same module.
    let output = result(
        r#"
const out = [];
const log = (...items) => out.push(items.join(':'));
const show = (result) => `${result.value}/${result.done}`;
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function* counter(limit) {
  try {
    for (let i = 0; i < limit; i++) {
      const sent = yield i;
      if (sent) log('sent', sent);
      await tick();
    }
    return 'end';
  } finally {
    log('finally');
  }
}
const proto = Object.getPrototypeOf(counter);
const generatorPrototype = proto.prototype;
const iteratorPrototype = Object.getPrototypeOf(generatorPrototype);
log('shape', typeof counter, Object.prototype.toString.call(proto), Object.prototype.toString.call(counter(1)), typeof iteratorPrototype[Symbol.asyncIterator], Object.getPrototypeOf(counter(0)) === counter.prototype, Object.getPrototypeOf(counter.prototype) === generatorPrototype, generatorPrototype.constructor === proto);
log('events-shape', Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}).prototype) === iteratorPrototype);

export let result = '';
(async () => {
const g = counter(3);
log('self', g[Symbol.asyncIterator]() === g);
// Requests queue while the body runs.
const pending = [g.next(), g.next('a'), g.next('b'), g.next(), g.next()];
const results = await Promise.all(pending);
log('queued', results.map(show).join(','));

const h = counter(5);
log('first', show(await h.next()));
log('return', show(await h.return(Promise.resolve('early'))));
log('after', show(await h.next()));

const k = counter(5);
await k.next();
try { await k.throw(new Error('boom')); } catch (error) { log('thrown', error.message); }
log('done', show(await k.next()));

const fresh = counter(2);
log('start-return', show(await fresh.return('r')));
const fresh2 = counter(2);
try { await fresh2.throw('t'); } catch (error) { log('start-throw', error); }
log('start-return-rejected', await counter(1).return(Promise.reject('no')).catch((reason) => `caught ${reason}`));

async function* rejecting() {
  yield 1;
  await Promise.reject(new Error('inner'));
}
const r = rejecting();
log('reject', show(await r.next()), await r.next().catch((error) => error.message), show(await r.next()));

async function* awaitsYield() {
  const value = yield Promise.resolve('resolved');
  log('received', value);
  try {
    yield 'try';
  } catch (error) {
    log('caught', error);
    yield 'recovered';
  }
}
const a = awaitsYield();
log('awaited', show(await a.next()), show(await a.next('x')), show(await a.throw('oops')), show(await a.next()));

const methods = {
  async *values() { yield 1; yield 2; },
};
class Holder {
  static async *pairs() { yield 'p'; }
}
const collected = [];
const it = methods.values();
for (let step = await it.next(); !step.done; step = await it.next()) collected.push(step.value);
log('methods', collected.join(','), show(await Holder.pairs().next()));
log('receiver', await counter.prototype.next.call({}).catch((error) => `${error.constructor.name}`));
result = out.join('|');
})();
"#,
    );
    assert_eq!(
        output,
        r#"shape:function:[object AsyncGeneratorFunction]:[object AsyncGenerator]:function:true:true:true|events-shape:true|self:true|sent:a|sent:b|finally|queued:0/false,1/false,2/false,end/true,undefined/true|first:0/false|finally|return:early/true|after:undefined/true|finally|thrown:boom|done:undefined/true|start-return:r/true|start-throw:t|start-return-rejected:caught no|reject:1/false:inner:undefined/true|received:x|caught:oops|awaited:resolved/false:try/false:recovered/false:undefined/true|methods:1,2:p/false|receiver:TypeError"#
    );
}

#[test]
fn async_iteration_matches_node() {
    let output = result(include_str!("tests/async_iteration.mjs"));
    assert_eq!(
        output,
        include_str!("tests/async_iteration.expected.txt").trim()
    );
}

#[test]
fn url_search_params_globals_link_the_compiled_node_source() {
    let compiled = compile_dynamic(
        "export const params = new URLSearchParams('a=1');",
        Path::new("params.js"),
    )
    .unwrap();
    assert_eq!(
        compiled
            .native_dependencies()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["node:url"]
    );
}

#[test]
fn native_dependency_metadata_tracks_imports_globals_and_shadowing() {
    for (source, expected) in [
        ("export const kind = typeof URL;", vec!["node:url"]),
        (
            "export const kind = typeof URLSearchParams;",
            vec!["node:url"],
        ),
        ("export const kind = typeof Buffer;", vec!["node:buffer"]),
        ("export const kind = process;", vec!["node:events"]),
        (
            "export const kind = globalThis['URL'];",
            vec!["node:buffer", "node:events", "node:url"],
        ),
        (
            "export const kind = global.URL;",
            vec!["node:buffer", "node:events", "node:url"],
        ),
        (
            "export function f(URL,globalThis) { return [URL,globalThis.URL]; }",
            vec![],
        ),
        ("export const read = fetch;", vec!["node:internal/fetch"]),
        (
            "export const stream = new ReadableStream();",
            vec!["node:stream/web"],
        ),
        (
            "export const thrown = typeof DOMException;",
            vec!["node:internal/per_context/domexception"],
        ),
        (
            "export class AppRequest extends global.Request {}",
            vec![
                "node:buffer",
                "node:events",
                "node:internal/fetch",
                "node:url",
            ],
        ),
        (
            "export function f(globalThis) { return globalThis.Response; }",
            vec![],
        ),
        ("export const kind = process.platform;", vec![]),
        (
            "export const literal = 'modules::load(realm, \"node:url\")';",
            vec![],
        ),
    ] {
        let module = compile_dynamic(source, Path::new("dependencies.js")).unwrap();
        assert_eq!(
            module
                .native_dependencies()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            expected,
            "{source}"
        );
    }
    let module = compile_sources(
        Path::new("main.js"),
        &mut |_, _, _| Some(PathBuf::from("native:math")),
        &mut |path| {
            Some(if path == Path::new("main.js") {
                ModuleSource {
                    text: "import value from 'math'; export {value};".into(),
                    kind: ModuleKind::EsModule,
                }
            } else {
                ModuleSource {
                    text: "native:math".into(),
                    kind: ModuleKind::Native,
                }
            })
        },
    )
    .unwrap();
    assert_eq!(
        module
            .native_dependencies()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["native:math"]
    );
    let crypto = compile_sources(
        Path::new("main.js"),
        &mut |_, _, _| Some(PathBuf::from("native:crypto")),
        &mut |path| {
            Some(if path == Path::new("main.js") {
                ModuleSource {
                    text: "import crypto from 'crypto'; export {crypto};".into(),
                    kind: ModuleKind::EsModule,
                }
            } else {
                ModuleSource {
                    text: "crypto".into(),
                    kind: ModuleKind::Native,
                }
            })
        },
    )
    .unwrap();
    assert_eq!(
        crypto
            .native_dependencies()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        // A native module links only itself: Node's crypto is a library
        // module, which requires the buffer module it returns Buffers of.
        vec!["crypto"]
    );
}

#[test]
fn bigint_typed_array_globals_compile_exact_native_values() {
    let output = result(
        r#"
        const signed = new BigInt64Array([9007199254740993n, -1n]);
        const unsigned = new BigUint64Array(signed.buffer);
        export const result = signed.join(',') + '|' + unsigned.join(',');
    "#,
    );
    assert_eq!(
        output,
        "9007199254740993,-1|9007199254740993,18446744073709551615"
    );
}

#[test]
fn ordinary_new_target_is_lexical_in_arrows_and_preserved_by_reflect_and_bind() {
    let output = result(
        r#"
        const out=[];
        function F(target = new.target) {
          out.push(target===new.target, new.target===undefined);
          const arrow=()=>new.target;
          function nested(){return new.target;}
          out.push(nested()===undefined);
          if (new.target) { this.arrow=arrow; this.target=new.target; }
          else return arrow;
        }
        const called=F();out.push(called()===undefined);
        const own=new F();out.push(own.arrow()===F,own.target===F);
        function Other(){}
        const reflected=Reflect.construct(F,[],Other);out.push(reflected.arrow()===Other,reflected instanceof Other);
        const bound=F.bind(null);const b=new bound();out.push(b.arrow()===F);
        const rb=Reflect.construct(bound,[],Other);out.push(rb.arrow()===Other);
        function outer(){const before=new.target;try{Reflect.construct(function(){throw 1;},[]);}catch(e){}return {same:new.target===before,arrow:()=>new.target};}
        const o=new outer();out.push(o.same,o.arrow()===outer,outer().arrow()===undefined);
        const obj={method(){return new.target;}};out.push(obj.method()===undefined);
        function Params(first=()=>new.target){return {read:first};}
        out.push(new Params().read()===Params);
        function FieldOwner() {
          class Fields { value=new.target; arrow=()=>new.target; static value=new.target; static {out.push(new.target===undefined);} }
          const f=new Fields();out.push(f.value===undefined,f.arrow()===undefined,Fields.value===undefined);
        }
        new FieldOwner();
        export const result=out.join('|');
    "#,
    );
    assert_eq!(
        output,
        "true|true|true|true|true|false|true|true|true|true|false|true|true|true|true|false|true|true|true|false|true|true|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn statically_known_project_values_compile_as_native_literals() {
    let options = CompileOptions {
        defines: std::collections::BTreeMap::from([
            (
                "VERSION".to_owned(),
                CompileValue::String("0.20.0".to_owned()),
            ),
            ("FLAG".to_owned(), CompileValue::Boolean(true)),
            ("COUNT".to_owned(), CompileValue::Number(4.0)),
            ("EMPTY".to_owned(), CompileValue::Null),
            ("MISSING".to_owned(), CompileValue::Undefined),
        ]),
        import_meta_env: Some(std::collections::BTreeMap::from([
            ("DEV".to_owned(), CompileValue::Boolean(false)),
            ("GLOBAL".to_owned(), CompileValue::GlobalThis),
        ])),
        ..CompileOptions::default()
    };
    let path = Path::new("main.ts");
    let module = compile_sources_with_options(
        path,
        &mut |_, _, _| None,
        &mut |_| {
            Some(ModuleSource {
                text: "export const values = [VERSION, FLAG, COUNT, EMPTY, MISSING, import.meta.env.DEV, import.meta.env.UNSET];".to_owned(),
                kind: ModuleKind::EsModule,
            })
        },
        &options,
    )
    .unwrap();
    let rust = module.rust_source();
    for literal in [
        "Value::str(\"0.20.0\")",
        "Value::Boolean(true)",
        "Value::Number(f64::from_bits(0x4010000000000000))",
        "Value::Null",
        "Value::Undefined",
        "Value::Boolean(false)",
    ] {
        assert!(rust.contains(literal), "missing {literal} in {rust}");
    }
    assert!(!rust.contains("ImportMeta is not compiled yet"));
}

#[test]
fn dotted_defines_replace_global_member_paths_only() {
    let options = CompileOptions {
        defines: std::collections::BTreeMap::from([(
            "process.env.NODE_ENV".to_owned(),
            CompileValue::String("production".to_owned()),
        )]),
        ..CompileOptions::default()
    };
    let module = compile_sources_with_options(
        Path::new("main.ts"),
        &mut |_, _, _| None,
        &mut |_| {
            Some(ModuleSource {
                text: "export const mode = process.env.NODE_ENV;
export const other = process.env.HOME;
export const optional = process?.env.NODE_ENV;
export const computed = process.env['NODE_ENV'];
export function local(process) { return process.env.NODE_ENV; }
export const call = (() => ({ env: {} }))().env.NODE_ENV;"
                    .to_owned(),
                kind: ModuleKind::EsModule,
            })
        },
        &options,
    )
    .unwrap();
    let rust = module.rust_source();
    assert_eq!(
        rust.matches("Value::str(\"production\")").count(),
        1,
        "{rust}"
    );
}

#[test]
fn global_this_defines_and_unimplemented_global_reads_keep_their_native_boundaries() {
    let options = CompileOptions {
        defines: std::collections::BTreeMap::from([(
            "global".to_owned(),
            CompileValue::GlobalThis,
        )]),
        ..CompileOptions::default()
    };
    let module = compile_sources_with_options(
        Path::new("main.ts"),
        &mut |_, _, _| None,
        &mut |_| {
            Some(ModuleSource {
                text: "export const value = global === globalThis;".to_owned(),
                kind: ModuleKind::EsModule,
            })
        },
        &options,
    )
    .unwrap();
    assert!(
        module
            .rust_source()
            .contains("ops::get_static_at(realm, &global")
    );

    let ambient = compile_dynamic(
        "export const value = missingApplicationHost;",
        Path::new("ambient-host.ts"),
    )
    .unwrap();
    assert!(
        ambient
            .rust_source()
            .contains("ops::get_global(realm, \"missingApplicationHost\")")
    );
    let process = compile_dynamic(
        "export const value = process.abort;",
        Path::new("unsupported-host.ts"),
    )
    .unwrap_err();
    assert!(
        process
            .iter()
            .any(|error| error.message.contains("process.abort is not compiled yet"))
    );
}

/// A React-like runtime that prints the calls JSX makes.
const JSX_RUNTIME: &str = "function show(type, props, key) {
  const children = props.children === undefined ? [] : [].concat(props.children);
  const attributes = Object.keys(props).filter((name) => name !== 'children').map((name) => name + '=' + props[name]);
  return '<' + (typeof type === 'string' ? type : type.name || 'Fragment') + (key === undefined ? '' : '#' + key) + (attributes.length ? ' ' + attributes.join(' ') : '') + '>' + children.join('') + '</>';
}
export function jsx(type, props, key) { return show(type, props, key); }
export function jsxs(type, props, key) { return 'many' + show(type, props, key); }
export const Fragment = { name: 'Fragment' };
export function createElement(type, props, ...children) { return show(type, Object.assign({}, props, children.length ? { children } : {})); }";

/// Compiles `entry` with modules from `files`, recording the requests made.
fn compile_files(
    entry: &str,
    files: &[(&str, &str)],
    options: &CompileOptions,
    requested: &mut Vec<String>,
) -> Result<DynamicModule, Vec<String>> {
    let files: std::collections::HashMap<PathBuf, String> = files
        .iter()
        .map(|(name, text)| (PathBuf::from(name), (*text).to_owned()))
        .collect();
    compile_sources_with_options(
        Path::new(entry),
        &mut |_, specifier, _| {
            requested.push(specifier.to_owned());
            let name = specifier.trim_start_matches("./");
            [name, &format!("{name}.ts"), &format!("{name}.js")]
                .into_iter()
                .map(PathBuf::from)
                .find(|path| files.contains_key(path))
        },
        &mut |path| {
            files.get(path).map(|text| ModuleSource {
                text: text.clone(),
                kind: ModuleKind::EsModule,
            })
        },
        options,
    )
    .map_err(|errors| errors.into_iter().map(|error| error.message).collect())
}

#[test]
fn jsx_compiles_to_the_automatic_runtime() {
    let mut requested = Vec::new();
    let module = compile_files(
        "component.tsx",
        &[
            (
                "component.tsx",
                "function Item({ label }: { label: string }) { return <li>{label}</li>; }
const rest = { title: 'x' };
export const result = <section id=\"a\" {...rest}>
  <Item label=\"one\" key=\"k\" />
  text &amp; more
  <>{[1, 2].map((n) => <b key={n}>{n}</b>)}</>
</section>;",
            ),
            ("react/jsx-runtime.js", JSX_RUNTIME),
        ],
        &CompileOptions::default(),
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(requested, ["react/jsx-runtime"]);
    assert_eq!(
        run(
            &module,
            "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); let text = show(&mut realm, &value); println!(\"{text}\");"
        ),
        "many<section id=a title=x><Item#k label=one></>text & more<Fragment><b#1>1</><b#2>2</></></>"
    );
}

#[test]
fn jsx_follows_the_configured_runtime() {
    let source = "export const element = <div><span /></div>;";
    let mut requested = Vec::new();
    let options = CompileOptions {
        jsx: Jsx::Automatic {
            import_source: "preact".to_owned(),
        },
        ..CompileOptions::default()
    };
    compile_files(
        "view.jsx",
        &[("view.jsx", source), ("preact/jsx-runtime.js", JSX_RUNTIME)],
        &options,
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(requested, ["preact/jsx-runtime"]);

    let mut requested = Vec::new();
    let options = CompileOptions {
        jsx: Jsx::Classic {
            factory: "React.createElement".to_owned(),
            fragment: "React.Fragment".to_owned(),
        },
        ..CompileOptions::default()
    };
    let classic = format!(
        "import * as React from './react';
{source}"
    );
    compile_files(
        "view.tsx",
        &[("view.tsx", &classic), ("react.js", JSX_RUNTIME)],
        &options,
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(requested, ["./react"]);
}

#[test]
fn typescript_imports_that_no_value_uses_are_dropped() {
    let source = "import { Shape, helper } from './shapes';
import { Options } from './only-types';
import Default, { type Named } from './defaults';
import './effects';
export { helper as again };
export const result: Shape = { size: helper() } as Shape;
export function measure(options: Options, named: Named): typeof Default { return 1; }";
    let files = [
        ("main.ts", source),
        (
            "shapes.ts",
            "export interface Shape { size: number } export function helper() { return 2; }",
        ),
        ("defaults.ts", "export default 1;"),
        ("effects.ts", "globalThis.effect = true;"),
    ];
    let mut requested = Vec::new();
    compile_files(
        "main.ts",
        &files,
        &CompileOptions::default(),
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(requested, ["./shapes", "./effects"]);

    // `verbatimModuleSyntax` keeps every import, as JavaScript does.
    let mut requested = Vec::new();
    let _ = compile_files(
        "main.ts",
        &files,
        &CompileOptions {
            verbatim_module_syntax: true,
            ..CompileOptions::default()
        },
        &mut requested,
    );
    for kept in ["./only-types", "./defaults"] {
        assert!(
            requested.iter().any(|request| request == kept),
            "{requested:?}"
        );
    }
}

#[test]
fn ambient_module_declarations_have_no_runtime_behavior() {
    assert_eq!(
        result(
            "declare module 'router' { interface Register { router: string } }
export const result = 'declared';"
        ),
        "declared"
    );
}

#[test]
fn import_meta_is_module_local_and_exposes_an_escaped_file_url() {
    let entry = std::env::current_dir().unwrap().join("Mark Text # 中文.js");
    let options = CompileOptions {
        import_meta_env: Some(std::collections::BTreeMap::from([
            ("DEV".to_owned(), CompileValue::Boolean(false)),
            ("GLOBAL".to_owned(), CompileValue::GlobalThis),
        ])),
        ..CompileOptions::default()
    };
    let module = compile_sources_with_options(
        &entry,
        &mut |_, _, _| None,
        &mut |path| {
            (path == entry).then(|| ModuleSource {
                text: "const meta = import.meta; meta.extra = 'ok'; export const result = [meta === import.meta, meta.url, meta.env.DEV, meta.extra, meta.env.GLOBAL === globalThis].join('|');".to_owned(),
                kind: ModuleKind::EsModule,
            })
        },
        &options,
    )
    .unwrap();
    let output = run(
        &module,
        "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); println!(\"{}\", show(&mut realm, &value));",
    );
    let parts = output.split('|').collect::<Vec<_>>();
    assert_eq!(parts.first(), Some(&"true"));
    assert_eq!(parts.get(2), Some(&"false"));
    assert_eq!(parts.get(3), Some(&"ok"));
    assert_eq!(parts.get(4), Some(&"true"));
    let url = parts.get(1).unwrap();
    assert!(url.starts_with("file:///"), "{url}");
    assert!(
        url.ends_with("/Mark%20Text%20%23%20%E4%B8%AD%E6%96%87.js"),
        "{url}"
    );
}

#[test]
fn strict_directives_in_arrows_and_methods_are_emitted() {
    let module = mixed(
        "main.cjs",
        &[(
            "main.cjs",
            "module.exports = { method() { return this; }, arrow: () => { 'use strict'; return this; } };",
        )],
    )
    .unwrap();
    assert_eq!(
        run(
            &module,
            "let object = ops::get(&mut realm, &exports, &Key::name(\"default\")).unwrap(); let method = ops::get(&mut realm, &object, &Key::name(\"method\")).unwrap(); let result = ops::call(&mut realm, &method, Value::Undefined, &[]).unwrap(); println!(\"{}\", show(&mut realm, &result));"
        ),
        "undefined"
    );
}

/// Compiles a mixed graph with `options`, recording the requests made.
fn mixed_with(
    entry: &str,
    files: &[(&str, &str)],
    options: &CompileOptions,
    requested: &mut Vec<String>,
) -> Result<DynamicModule, Vec<crate::Diagnostic>> {
    let files: std::collections::HashMap<PathBuf, ModuleSource> = files
        .iter()
        .map(|(name, text)| {
            let kind = if name.ends_with(".cjs") {
                ModuleKind::CommonJs
            } else {
                ModuleKind::EsModule
            };
            let text = (*text).to_owned();
            (PathBuf::from(name), ModuleSource { text, kind })
        })
        .collect();
    compile_sources_with_options(
        Path::new(entry),
        &mut |_, name, _| {
            requested.push(name.to_owned());
            let path = PathBuf::from(name.strip_prefix("./").unwrap_or(name));
            (files.contains_key(&path) || name.ends_with(".unreadable")).then_some(path)
        },
        &mut |path| files.get(path).cloned(),
        options,
    )
}

fn print_result(module: &DynamicModule) -> String {
    run(
        module,
        "let value = ops::get(&mut realm, &exports, &Key::name(\"result\")).unwrap(); let text = show(&mut realm, &value); println!(\"{text}\");",
    )
}

#[test]
fn requests_in_try_blocks_that_cannot_load_throw_when_they_run() {
    let mut requested = Vec::new();
    let module = mixed_with(
        "main.ts",
        &[
            (
                "main.ts",
                "import loader from './loader.cjs'; export const result = loader;",
            ),
            (
                "loader.cjs",
                "let binding = null, failures = [];
try { binding = require('./native.unreadable'); } catch (error) { failures.push(error.code + ' ' + error.message); }
try { binding = require('platform-package'); } catch (error) { failures.push(error.code); }
try { binding = require('./fallback.cjs'); } catch (error) { failures.push('unexpected'); }
module.exports = failures.concat(binding).join('|');",
            ),
            ("fallback.cjs", "module.exports = 'fallback';"),
        ],
        &CompileOptions::default(),
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(
        print_result(&module),
        "MODULE_NOT_FOUND Cannot find module './native.unreadable'|MODULE_NOT_FOUND|fallback"
    );

    // Outside `try` blocks the build warns, and they throw when they run.
    let mut requested = Vec::new();
    let module = mixed_with(
        "main.ts",
        &[
            (
                "main.ts",
                "import code from './load.cjs'; export const result = code;",
            ),
            (
                "load.cjs",
                "function load() { return require('platform-package'); } let code; try { code = load(); } catch (error) { code = error.code; } module.exports = code;",
            ),
        ],
        &CompileOptions::default(),
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(
        module.warnings()[0].message,
        "cannot resolve module 'platform-package'; requiring it throws MODULE_NOT_FOUND when it runs"
    );
    assert_eq!(print_result(&module), "MODULE_NOT_FOUND");
}

/// Requests computed when the application runs: CommonJS `require.resolve`
/// answers for the modules the file requires by name and throws
/// MODULE_NOT_FOUND for others, and a createRequire require throws
/// MODULE_NOT_FOUND, each with a build warning.
#[test]
fn computed_requests_resolve_compiled_modules_and_throw_for_others() {
    let mut requested = Vec::new();
    let module = mixed_with(
        "main.ts",
        &[
            (
                "main.ts",
                "import value from './entry.cjs'; export const result = value;",
            ),
            (
                "entry.cjs",
                "require('./dep.cjs'); const known = './dep.cjs', other = 'elsewhere'; \
                 let missing; try { require.resolve(other); } catch (error) { missing = error.code; } \
                 const created = require('./created.mjs'); \
                 module.exports = [require.resolve(known).endsWith('dep.cjs'), missing, created.code].join('|');",
            ),
            ("dep.cjs", "module.exports = 1;"),
            (
                "created.mjs",
                "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url); const name = 'computed-' + 'package'; export let code; try { require(name); } catch (error) { code = error.code; }",
            ),
        ],
        &CompileOptions::default(),
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let warnings: std::collections::BTreeSet<&str> = module
        .warnings()
        .iter()
        .map(|warning| warning.message.as_str())
        .collect();
    assert_eq!(
        warnings.into_iter().collect::<Vec<_>>(),
        [
            "this require is computed when the application runs and throws MODULE_NOT_FOUND then; only modules required by name are compiled",
            "this require.resolve is computed when the application runs and throws MODULE_NOT_FOUND then for modules the file does not require by name",
        ]
    );
    assert_eq!(
        print_result(&module),
        "true|MODULE_NOT_FOUND|MODULE_NOT_FOUND"
    );
}

#[test]
fn branches_that_build_time_defines_rule_out_make_no_requests() {
    let options = CompileOptions {
        defines: std::collections::BTreeMap::from([(
            "process.env.NODE_ENV".to_owned(),
            CompileValue::String("production".to_owned()),
        )]),
        ..CompileOptions::default()
    };
    let mut requested = Vec::new();
    let module = mixed_with(
        "main.ts",
        &[
            (
                "main.ts",
                "import entry from './index.cjs'; export const result = entry;",
            ),
            (
                "index.cjs",
                "if (process.env.NODE_ENV === 'production') {
  module.exports = require('./production.cjs');
} else {
  module.exports = require('./development.cjs');
}
const tools = process.env.NODE_ENV !== 'production' ? require('./development-tools.cjs') : null;
const debug = process.env.NODE_ENV == 'development' && require('./debug.cjs');
const kept = process.env.NODE_ENV === 'production' || require('./never.cjs');",
            ),
            ("production.cjs", "module.exports = 'production';"),
        ],
        &options,
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(requested, ["./index.cjs", "./production.cjs"]);
    assert_eq!(print_result(&module), "production");

    // Without the define, both branches are compiled.
    let mut requested = Vec::new();
    let _ = mixed_with(
        "index.cjs",
        &[(
            "index.cjs",
            "module.exports = process.env.NODE_ENV === 'production' ? require('./a.cjs') : require('./b.cjs');",
        )],
        &CompileOptions::default(),
        &mut requested,
    );
    assert_eq!(requested, ["./a.cjs", "./b.cjs"]);
}

#[test]
fn module_exports_can_be_defined_as_an_accessor() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import styles from './styles.cjs'; export const result = styles.red;",
            ),
            (
                "styles.cjs",
                "function assemble() { return { red: 'assembled' }; }
Object.defineProperty(module, 'exports', { enumerable: true, get: assemble });",
            ),
        ],
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(print_result(&module), "assembled");
}

#[test]
fn process_parent_mask_report_and_standard_input() {
    // The program runs as a child of this test process.
    assert_eq!(
        result(&format!(
            "export const result = [
  process.ppid === {},
  process.umask(), process.umask(0o22), process.umask('077'), process.umask(),
  process.report.getReport().header.platform, process.report.getReport().header.processId === process.pid,
  process.stdin.fd, process.stdin.pause() === process.stdin,
  process.stdout.columns, process.stderr.rows,
].join('|');",
            std::process::id()
        )),
        "true|0|0|18|63|win32|true|0|true||"
    );
}

#[test]
fn sources_bundled_into_commonjs_have_require_and_the_bundle_paths() {
    let options = CompileOptions {
        bundle: Some("dist/main/index.js".to_owned()),
        ..CompileOptions::default()
    };
    let mut requested = Vec::new();
    let module = mixed_with(
        "main.ts",
        &[
            (
                "main.ts",
                "const top = require('./dep.cjs').value;
function nested() { return (() => require('./dep.cjs').value + 1)(); }
class Holder { read() { return require('./dep.cjs').value + 2; } }
export const result = [top, nested(), new Holder().read(),
  __dirname.split(String.fromCharCode(92)).slice(-2).join('/') === 'dist/main',
  __filename.split(String.fromCharCode(92)).slice(-3).join('/') === 'dist/main/index.js'].join('|');",
            ),
            ("dep.cjs", "module.exports = { value: 40 };"),
        ],
        &options,
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(requested, ["./dep.cjs"; 3]);
    assert_eq!(print_result(&module), "40|41|42|true|true");

    // Without a bundle, an unbound `require` is an ordinary global.
    let mut requested = Vec::new();
    mixed_with(
        "main.ts",
        &[("main.ts", "export const result = typeof require;")],
        &CompileOptions::default(),
        &mut requested,
    )
    .unwrap();
    assert!(requested.is_empty());
}

/// The application's CommonJS modules and its installed packages have the
/// paths of their places in the folder the application runs from.
#[test]
fn application_commonjs_modules_take_their_paths_in_the_running_application() {
    let options = CompileOptions {
        application: Some(PathBuf::new()),
        ..CompileOptions::default()
    };
    let mut requested = Vec::new();
    let module = mixed_with(
        "main.ts",
        &[
            (
                "main.ts",
                "import own from './lib/dep.cjs';
import installed from './node_modules/pkg/index.cjs';
const folder = process.execPath.slice(0, process.execPath.lastIndexOf(String.fromCharCode(92)));
const slash = String.fromCharCode(92);
export const result = [own.dir === folder + slash + 'lib',
  own.file === folder + slash + 'lib' + slash + 'dep.cjs',
  installed.dir.startsWith(folder)].join('|');",
            ),
            (
                "lib/dep.cjs",
                "module.exports = { dir: __dirname, file: __filename };",
            ),
            (
                "node_modules/pkg/index.cjs",
                "module.exports = { dir: __dirname };",
            ),
        ],
        &options,
        &mut requested,
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(print_result(&module), "true|true|true");
}

#[test]
fn new_names_a_callee_that_is_not_a_constructor_as_v8_does() {
    let output = result(
        r#"
        const names = [];
        const o = { make: () => 1, inner: {} };
        const Intl2 = {};
        for (const f of [
            () => new o.make(),
            () => new o.inner.Missing(),
            () => new (o.make)(),
            () => new Intl2.DisplayNames("en"),
            () => new o["make"](),
        ]) {
            try { f(); } catch (error) { names.push(error.message); }
        }
        export const result = names.join("|");
        "#,
    );
    assert_eq!(
        output.trim(),
        "o.make is not a constructor|o.inner.Missing is not a constructor|o.make is not a constructor|Intl2.DisplayNames is not a constructor|o.make is not a constructor"
    );
}

#[test]
fn modules_too_large_for_one_crate_compile_in_parts_that_behave_the_same() {
    let files = [
        (
            "main.mjs",
            "import dep from './dep.cjs';\n\
             const out = []; let counter = 0;\n\
             function add(a, b = 2) { counter++; return a + b + arguments.length; }\n\
             function Outer() { return () => this * 2 + arguments.length; }\n\
             const obj = { n: 3, get() { return [1].map(() => this.n + arguments.length)[0]; } };\n\
             function* gen(k) { yield k; yield counter; }\n\
             async function later(x) { await null; return x * 2; }\n\
             class Base { #secret = 1; constructor(v) { this.v = v; } read() { return this.#secret + this.v; } static make() { return new Base(4); } }\n\
             class Derived extends Base { constructor() { super(5); this.w = () => this.v; } read() { return super.read() + 1; } twice() { return [1].map(() => this.v)[0] * 2; } }\n\
             class Field { f = () => this.g; g = 6; }\n\
             const meta = function () { return typeof import.meta.url; };\n\
             const named = function self(n) { return n ? self(n - 1) + 1 : 0; };\n\
             out.push(add(1), add(1, 1), Outer.call(5, 1, 2)(), obj.get(), [...gen(7)].join(':'), Base.make().read());\n\
             out.push(new Derived().read(), new Derived().w(), new Derived().twice(), new Field().f(), meta(), named(3));\n\
             out.push(dep.value(), dep.load());\n\
             later(21).then((value) => { out.push(value); });\n\
             export const result = out;",
        ),
        (
            "dep.cjs",
            "const path = 'x';\n\
             function value() { return path + require('./leaf.cjs').leaf; }\n\
             module.exports = { value, load: () => typeof require };",
        ),
        ("leaf.cjs", "exports.leaf = 'L';"),
    ];
    let compile = |split_module_bytes| {
        let files = files
            .iter()
            .map(|(name, text)| {
                let kind = if name.ends_with(".cjs") {
                    ModuleKind::CommonJs
                } else {
                    ModuleKind::EsModule
                };
                let text = (*text).to_owned();
                (PathBuf::from(name), ModuleSource { text, kind })
            })
            .collect::<std::collections::HashMap<_, _>>();
        compile_sources_with_options(
            Path::new("main.mjs"),
            &mut |_, name, _| Some(PathBuf::from(name.strip_prefix("./").unwrap_or(name))),
            &mut |path| files.get(path).cloned(),
            &CompileOptions {
                split_module_bytes,
                ..CompileOptions::default()
            },
        )
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
    };
    let expected = "4,4,12,3,7:2,5,7,5,10,6,string,3,xL,function,42";
    let whole = compile(None);
    assert!(
        whole
            .module_rust_parts()
            .iter()
            .all(|parts| parts.len() == 1)
    );
    // A limit above every module keeps them whole.
    assert_eq!(compile(Some(usize::MAX)), whole);
    assert_eq!(print_result(&whole), expected);
    // Each function body becomes a Rust function; each part ends after one
    // once it is large enough, and later parts use the earlier parts' items.
    for limit in [1, 8000] {
        let split = compile(Some(limit));
        let parts = split.module_rust_parts();
        assert!(parts.iter().any(|parts| parts.len() > 2), "{limit}");
        let main = parts.iter().find(|parts| parts.len() > 2).unwrap();
        assert!(main[0].contains("pub fn P"));
        assert!(main[0].contains("pub static P"));
        assert!(main.last().unwrap().contains("fn m"));
        assert_eq!(print_result(&split), expected, "{limit}");
    }
}

/// TypeScript enums fold constant member expressions as `tsc` does
/// (shifts and bitwise operators in 32 bits, string concatenation and
/// templates, earlier members by name or through the enum), number the
/// members after a folded one, run other initializers at run time, and name
/// numeric values in reverse.
#[test]
fn typescript_enums_fold_constant_members() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "const enum Flags { None = 0, A = 1 << 0, B = 1 << 1, AB = A | B, C = Flags.AB << 2, Not = ~0, Big = 1 << 31, Shr = -16 >> 2, Ushr = -16 >>> 28 } \
             enum Text { Hello = 'hi', World = `${Hello} world`, Joined = 'a' + 'b' } \
             enum Auto { X = 3, Y, Z } \
             function compute() { return 40; } \
             enum Runtime { Value = compute(), Next = 2 } \
             enum Frac { Half = 0.5 } \
             export const result = [Flags.AB, Flags.C, Flags[3], Flags.Big, Flags.Shr, Flags.Ushr, Flags.Not, Text.World, Text.Joined, Text['hi'] === undefined, Auto.Z, Auto[5], Runtime.Value, Runtime[40], Runtime.Next, Frac[0.5], Frac.Half].join('|');",
        )]),
        "3|12|AB|-2147483648|-4|15|-1|hi world|ab|true|5|Z|40|Value|2|Half|0.5"
    );
}

/// TypeScript namespaces are objects of their exported declarations, bound
/// before their bodies run: nested and dotted namespaces, merged namespace
/// declarations, and namespaces merged into a class, an enum and a
/// function. Unexported members stay private; namespaces of types only
/// have no value.
#[test]
fn typescript_namespaces_are_objects_of_their_exports() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "export namespace Shapes { \
               const secret = 2; \
               export const unit = secret * 2; \
               export function area(side: number) { return side * side * unit; } \
               export class Box { size() { return area(1); } } \
               export enum Kind { Square = 1 } \
               export namespace Inner { export const deep = unit + 1; } \
               export interface Type { x: number } \
               export type Alias = string; \
             } \
             export namespace Shapes { export const more = Shapes.unit + 10; } \
             namespace Only.Types { export interface X { y: number } } \
             namespace A.B.C { export const v = 7; } \
             class Color { constructor(public name: string) {} } \
             namespace Color { export const red = new Color('red'); export function of(name: string) { return new Color(name); } } \
             enum Mode { On = 1 } \
             namespace Mode { export function label(mode: Mode) { return Mode[mode]; } } \
             function greet() { return 'hi'; } \
             namespace greet { export const times = 2; } \
             export const result = [Shapes.unit, Shapes.area(3), new Shapes.Box().size(), Shapes.Kind.Square, Shapes.Inner.deep, Shapes.more, A.B.C.v, Color.red.name, Color.of('blue').name, Mode.label(Mode.On), greet.times, greet(), 'secret' in Shapes, Object.keys(Shapes).join(',')].join('|');",
        )]),
        "4|36|4|1|5|14|7|red|blue|On|2|hi|false|unit,area,Box,Kind,Inner,more"
    );
}

/// Node's `import.meta.filename` and `import.meta.dirname` name the file
/// `import.meta.url` names and its folder, with Windows separators.
#[test]
fn import_meta_names_the_module_file_and_folder() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "const slash = String.fromCharCode(92); const file = import.meta.filename; \
             export const result = [file.endsWith(slash + 'main.ts'), import.meta.dirname === file.slice(0, file.lastIndexOf(slash)), import.meta.url.startsWith('file:///') && import.meta.url.endsWith('/main.ts'), file.includes('/')].join('|');",
        )]),
        "true|true|true|false"
    );
}

/// Every function that reads `import.meta`, async ones included, sees the
/// module's one object.
#[test]
fn functions_and_async_functions_share_import_meta() {
    assert_eq!(
        project_result(&[(
            "main.ts",
            "const first = () => import.meta; function second() { return import.meta; } \
             async function later() { await null; return import.meta; } \
             export const result = [first() === second(), typeof later, typeof import.meta.url].join('|');",
        )]),
        "true|function|string"
    );
}

/// `import()` of a specifier computed at run time loads the candidate whose
/// path ends it: the paths a variable is given (as VS Code's window script
/// gives `workbenchUrl` a relative path or an absolute URL), conditionals
/// and templates of literals. Candidates that do not resolve are dropped,
/// and a specifier that names none rejects with `ERR_MODULE_NOT_FOUND`.
#[test]
fn computed_dynamic_imports_load_the_candidate_their_specifier_names() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "let url: string; \
                 if ((globalThis as any).pick) { url = './views/b.ts'; } else { url = `file:///app/out/${'views'}/b.ts`; } \
                 const load = (name: string) => import(name === 'a' ? './views/a.ts' : './views/b.ts'); \
                 const lost = (globalThis as any).pick ? './views/a.ts' : './views/none.ts'; \
                 Promise.all([import(url), load('a'), load('b'), import(`./views/${'b'}.ts`)]).then((all) => console.log(all.map((ns) => ns.name).join(','))); \
                 import(lost).catch((error) => console.log('missing', error.code));",
            ),
            ("views/a.ts", "export const name = 'A';"),
            ("views/b.ts", "export const name = 'B';"),
        ],
    )
    .unwrap();
    assert_eq!(module.entry_count(), 3);
    assert_eq!(
        run(&module, "let _ = exports; let _ = Key::name(\"name\");"),
        "missing ERR_MODULE_NOT_FOUND\nB,A,B,B"
    );
    let unknown = mixed(
        "main.ts",
        &[(
            "main.ts",
            "const lost = (globalThis as any).pick ? './a.ts' : './b.ts'; import(lost).catch((error) => console.log('rejected', error.code));",
        )],
    )
    .unwrap();
    assert!(unknown.warnings().iter().any(|warning| {
        warning
            .message
            .contains("no module this dynamic import can name resolves")
    }));
    assert_eq!(
        run(&unknown, "let _ = exports; let _ = Key::name(\"name\");"),
        "rejected ERR_MODULE_NOT_FOUND"
    );
}

/// A computed import that reads a property or an environment variable takes
/// the values the program gives that property anywhere, following
/// properties given other properties: VS Code's `bootstrap-fork` imports
/// the entry point its parent puts in `VSCODE_ESM_ENTRYPOINT`. Values that
/// name no module are left out, and a site that names none rejects.
#[test]
fn computed_imports_take_the_values_programs_give_properties() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "import { proc } from './second.ts'; import './starter.ts'; \
                 process.env.APP_ENTRY = proc.entryPoint; \
                 const entry = process.env['APP_ENTRY']; \
                 import([`./${entry}.ts`].join('/')).then((ns) => console.log(ns.name)); \
                 import(`./${process.env.NOWHERE}.ts`).catch((error) => console.log(error.code));",
            ),
            (
                "starter.ts",
                "export const config = { entryPoint: 'workers/one' }; export const other = { entryPoint: 'unrelated' };",
            ),
            (
                "second.ts",
                "export const watcher = { moduleId: 'workers/two' }; export const proc = { entryPoint: watcher.moduleId };",
            ),
            ("workers/one.ts", "export const name = 'one';"),
            ("workers/two.ts", "export const name = 'two';"),
        ],
    )
    .unwrap();
    assert_eq!(module.entry_count(), 3);
    let warnings: Vec<&str> = module
        .warnings()
        .iter()
        .map(|warning| warning.message.as_str())
        .collect();
    assert_eq!(
        warnings,
        [
            "no module this dynamic import can name resolves; it rejects with ERR_MODULE_NOT_FOUND when it runs"
        ]
    );
    assert_eq!(
        run(&module, "let _ = exports; let _ = Key::name(\"name\");"),
        "ERR_MODULE_NOT_FOUND\ntwo"
    );
}

/// Variables whose values name each other combine without bound: twenty
/// variables of three values each spell 3^20 paths. Their computed imports
/// are unknown past a budget instead of being spelled out.
#[test]
fn computed_imports_over_combinatorial_variables_stay_bounded() {
    let mut source = String::from("let v0 = 'a'; v0 = 'b'; v0 = 'c';\n");
    for index in 1..=20 {
        let previous = index - 1;
        source.push_str(&format!(
            "let v{index} = `${{v{previous}}}a`; v{index} = `${{v{previous}}}b`; v{index} = `${{v{previous}}}c`;\n"
        ));
    }
    source.push_str("const holder = {}; holder.entry = v20;\nimport(`./${v20}.ts`).catch((error) => console.log(error.code));\n");
    let started = std::time::Instant::now();
    let module = mixed("main.ts", &[("main.ts", &source)]).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
    let warnings: Vec<&str> = module
        .warnings()
        .iter()
        .map(|warning| warning.message.as_str())
        .collect();
    assert_eq!(
        warnings,
        [
            "this dynamic import names no module the build can find; it rejects with ERR_MODULE_NOT_FOUND when it runs"
        ]
    );
}

/// `using` and `await using` declarations lower to blocks that dispose
/// their resources through the Node library's disposal context, which the
/// program then links.
#[test]
fn using_declarations_link_the_disposal_context() {
    let module = mixed(
        "main.ts",
        &[(
            "main.ts",
            "export function run(log: string[]) { using a = { [Symbol.dispose]() { log.push('a'); } }; { using b = null; } log.push('body'); }              export async function later() { await using c = { async [Symbol.asyncDispose]() {} }; }",
        )],
    )
    .unwrap();
    assert!(
        module
            .native_dependencies()
            .contains("node:internal/using_ctx")
    );
    assert!(!module.rust_source().contains("not compiled yet"));
}

/// TypeScript expression forms VS Code uses: instantiation expressions as
/// values, non-null assertions ending optional chains, assignments through
/// `as` and `!`, `super` property assignments (through a setter, and as a
/// data property on `this`), and `import X = Y.Z` aliases, exported or
/// local, with aliases used only in types elided.
#[test]
fn typescript_expression_forms_and_import_aliases() {
    assert_eq!(
        project_result(&[
            (
                "severity.ts",
                "enum Severity { Ignore = 0, Info = 1, Warning = 2 } export default Severity;",
            ),
            (
                "notification.ts",
                "import BaseSeverity from 'severity.ts'; export import Severity = BaseSeverity;",
            ),
            (
                "main.ts",
                "import { Severity } from 'notification.ts'; \
                 function make<T>(): number { return 1; } const made = make<number>; \
                 const holder: any = { inner: { id: 4 } }; const chained = holder?.inner!.id; const missing = (undefined as any)?.inner!; \
                 class Base { _v = 0; set v(x: number) { this._v = x * 2; } get v() { return this._v; } } \
                 class Derived extends Base { store(x: number) { super.v = x; super['plain'] = 3; super['v'] = super.v + 1; return [this._v, (this as any).plain, Object.hasOwn(this, 'v')].join(','); } } \
                 let loose: any = 1; (loose as unknown) = 5; const target: any = {}; (target.key as unknown) = 7; (target['other']!) = 8; \
                 namespace Space { export const value = 9; export namespace Inner { export const deep = 10; } export interface Shape { size: number } } \
                 import Deep = Space.Inner; import ShapeAlias = Space.Shape; enum Local { One = 1 } import Alias = Local; \
                 const shape: ShapeAlias = { size: 1 }; \
                 export const result = [made(), chained, missing, new Derived().store(2), loose, target.key, target.other, Deep.deep, Alias.One, Severity.Warning, Severity[1], shape.size].join('|');",
            ),
        ]),
        "1|4||10,3,false|5|7|8|10|1|2|Info|1"
    );
}

/// String literals naming a package and a script in it side by side, as
/// script loaders name them, make that script a lazy entry of a graph that
/// compiles package scripts, with its key; other pairs are left out.
#[test]
fn package_scripts_named_by_literal_pairs_are_lazy_entries() {
    let files = [
        (
            "main.ts",
            "declare function load(a: string, b: string): void; \
             export const go = () => load('umd', 'main.js'); load('missing', 'x.js'); load('umd', 'readme.md');",
        ),
        ("umd/main.js", "var umdValue = 42;"),
    ];
    let options = CompileOptions {
        package_scripts: true,
        ..CompileOptions::default()
    };
    let mut requested = Vec::new();
    let module = mixed_with("main.ts", &files, &options, &mut requested).unwrap();
    assert_eq!(
        module.scripts(),
        [("node_modules/umd/main.js".to_owned(), 1)]
    );
    assert_eq!(module.entry_count(), 2);
    assert!(requested.iter().any(|request| request == "missing/x.js"));
    assert!(!requested.iter().any(|request| request.ends_with(".md")));
    let mut requested = Vec::new();
    let plain = mixed_with(
        "main.ts",
        &files,
        &CompileOptions::default(),
        &mut requested,
    )
    .unwrap();
    assert!(plain.scripts().is_empty());
    assert_eq!(plain.entry_count(), 1);
}

/// In a graph that constructs workers, string literals given to calls and
/// constructors that are paths of scripts make the scripts they name lazy
/// entries, keyed by those paths; a graph without workers looks none up.
#[test]
fn scripts_named_by_paths_are_lazy_entries_of_graphs_with_workers() {
    let files = [
        (
            "main.ts",
            "declare class Worker { constructor(url: string | URL, options?: object); }              declare function asUri(path: string): string;              export const start = () => new Worker(asUri('workers/editor.js'), { type: 'module' });              export const near = () => new Worker(new URL('./near.js', import.meta.url));              export const names = [asUri('missing.js'), asUri('readme.md'), asUri('https://example.com/x.js')];",
        ),
        ("workers/editor.js", "export const editor = 1;"),
        ("near.js", "export const near = 2;"),
        (
            "plain.ts",
            "declare function asUri(path: string): string; export const uri = asUri('workers/editor.js');",
        ),
    ];
    let options = CompileOptions {
        package_scripts: true,
        ..CompileOptions::default()
    };
    let mut requested = Vec::new();
    let module = mixed_with("main.ts", &files, &options, &mut requested).unwrap();
    assert_eq!(
        module.scripts(),
        [
            ("workers/editor.js".to_owned(), 1),
            ("near.js".to_owned(), 2)
        ]
    );
    assert_eq!(module.entry_count(), 3);
    assert!(requested.iter().any(|request| request == "missing.js"));
    assert!(
        !requested
            .iter()
            .any(|request| request.ends_with(".md") || request.contains(':'))
    );
    let mut requested = Vec::new();
    let plain = mixed_with("plain.ts", &files, &options, &mut requested).unwrap();
    assert!(plain.scripts().is_empty());
    assert!(
        !requested
            .iter()
            .any(|request| request.ends_with("editor.js"))
    );
}

/// A graph that registers service workers looks up their scripts as it
/// does workers', by the paths before a query, also in template literals.
#[test]
fn service_worker_scripts_are_lazy_entries_named_without_their_query() {
    let files = [
        (
            "main.js",
            "export const start = (version) => navigator.serviceWorker.register(encodeURI(`pre/sw.js?v=${version}`), { type: 'module' });              export const other = () => navigator.serviceWorker.register('pre/other.js?x=1#top');              export const skipped = (name) => `${name}.js`;",
        ),
        ("pre/sw.js", "self.addEventListener('fetch', () => {});"),
        ("pre/other.js", "export const other = 1;"),
    ];
    let options = CompileOptions {
        package_scripts: true,
        ..CompileOptions::default()
    };
    let mut requested = Vec::new();
    let module = mixed_with("main.js", &files, &options, &mut requested).unwrap();
    assert_eq!(
        module.scripts(),
        [("pre/sw.js".to_owned(), 1), ("pre/other.js".to_owned(), 2)]
    );
}

/// Closures capture values, not the types their annotations name: a
/// parameter default's arrow typed with the function's type parameter, as
/// VS Code's notebook `diff` has, captures nothing.
#[test]
fn closures_capture_values_not_the_types_their_annotations_name() {
    assert_eq!(
        result(
            "function diff<T>(before: T[], equal: (a: T, b: T) => boolean = (a: T, b: T) => a === b): T[] { const result: T[] = []; function push(value: T): void { result.push(value); } for (const value of before) { if (equal(value, value)) { push(value); } } return result; } export const result = diff([1, 2]).join(',');"
        ),
        "1,2"
    );
}

/// Names declared only with `declare` read the global at run time, as the
/// TypeScript they come from expects; none is bound in the emitted program.
#[test]
fn ambient_declarations_name_globals() {
    assert_eq!(
        result(
            "declare function importScripts(...urls: string[]): void; declare const ambientValue: number; declare class AmbientClass {} declare var ambientVar: string; declare let ambientCounter: number; globalThis.ambientValue = 5; globalThis.ambientCounter = 1; ambientCounter += 1; function probe() { return typeof importScripts; } export const result = [typeof importScripts, probe(), typeof AmbientClass, typeof ambientVar, ambientValue, (() => ambientValue + 1)(), globalThis.ambientCounter].join(',');"
        ),
        "undefined,undefined,undefined,undefined,5,6,2"
    );
}

/// Modules that await at top level evaluate as async functions: the entry
/// runs on after its awaits settle, and a dynamic import of such a module
/// resolves once its body finishes, as in Node.
#[test]
fn top_level_await_suspends_module_bodies_and_dynamic_imports() {
    let module = mixed(
        "main.ts",
        &[
            (
                "main.ts",
                "console.log('main start'); const ns = await import('lazy.ts'); \
                 console.log('main got', ns.answer, ns.late); \
                 for await (const value of [Promise.resolve(1), 2]) console.log('for await', value); \
                 console.log('main end');",
            ),
            (
                "lazy.ts",
                "console.log('lazy start'); export const answer = 42; export let late = 'pending'; \
                 await null; late = 'settled'; console.log('lazy end');",
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        run(&module, "let _ = exports; let _ = Key::name(\"answer\");"),
        "main start\nlazy start\nlazy end\nmain got 42 settled\nfor await 1\nfor await 2\nmain end"
    );
}
