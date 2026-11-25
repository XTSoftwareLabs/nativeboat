use super::*;

#[test]
fn async_hooks_resource_surface_compiles_as_a_public_node_module() {
    let entry = file_path("nativeboat/async-hooks-integration");
    let _module = compile_sources(
        &entry,
        &mut |from, name, _| builtin(name).or_else(|| internal(from, name)),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "const { AsyncResource } = require('async_hooks'); const resource = new AsyncResource('test'); module.exports = resource.runInAsyncScope(() => resource.asyncId() > 0);".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));

    assert_eq!(
        builtin("async_hooks"),
        Some(PathBuf::from("node:async_hooks"))
    );
    let async_hooks = source(Path::new("node:async_hooks.js")).unwrap();
    assert!(matches!(async_hooks.kind, ModuleKind::CommonJs));
    assert!(async_hooks.text.contains("class AsyncResource"));
}

#[test]
fn buffer_file_lazy_export_resolves_to_the_node_file_implementation() {
    let entry = file_path("nativeboat/buffer-file-integration");
    compile_sources(
        &entry,
        &mut |from, name, _| builtin(name).or_else(|| internal(from, name)),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "const { File } = require('buffer'); module.exports = new File(['text'], 'note.txt').name;".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));

    assert_eq!(builtin("buffer"), Some(PathBuf::from("node:buffer")));
    assert!(source(Path::new("node:internal/file.js")).is_some());
}

#[test]
fn filesystem_promises_source_graph_compiles_as_a_public_module() {
    let entry = file_path("nativeboat/fs-integration");
    let result = compile_sources(
        &entry,
        &mut |from, name, _| internal(from, name),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "'use strict'; module.exports = require('fs/promises');".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    );
    let module = result.unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(builtin("fs"), Some(PathBuf::from("node:fs")));
    assert_eq!(
        builtin("fs/promises"),
        Some(PathBuf::from("node:fs/promises"))
    );
    assert!(module.native_dependencies().contains("internal/binding"));
    let report = format!(
        "Native dependencies (not a public acceptance receipt): {:#?}\n",
        module.native_dependencies()
    );
    std::fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/task119p-fs-source-inventory.txt"),
        &report,
    )
    .unwrap();
    println!("{report}");
}

#[test]
fn filesystem_encoding_validator_is_bundled_and_exported() {
    let entry = file_path("nativeboat/fs-encoding-integration");
    compile_sources(
        &entry,
        &mut |from, name, _| internal(from, name),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "const { validateEncoding } = require('internal/validators'); validateEncoding('text', 'utf8'); module.exports = typeof validateEncoding;".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));

    let validators = source(Path::new("node:internal/validators.js")).unwrap();
    assert!(validators.text.contains("const validateEncoding ="));
    assert!(validators.text.contains("validateEncoding,"));
}

#[test]
fn util_source_graph_compiles_as_a_public_module() {
    let entry = file_path("nativeboat/util-integration");
    let result = compile_sources(
        &entry,
        &mut |from, name, _| internal(from, name),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "'use strict'; module.exports = require('util');".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    );
    let module = result.unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(builtin("util"), Some(PathBuf::from("node:util")));
    assert!(
        module
            .native_dependencies()
            .contains("internal/util/inspect")
    );
    for dependency in [
        "node:internal/util/comparisons.js",
        "node:internal/util/debuglog.js",
        "node:internal/mime.js",
        "node:internal/util/parse_args/parse_args.js",
        "node:internal/util/diff.js",
    ] {
        assert!(matches!(
            source(Path::new(dependency)).map(|source| source.kind),
            Some(ModuleKind::CommonJs)
        ));
    }
}

#[test]
fn bundled_util_parse_args_implementation_compiles() {
    let entry = file_path("nativeboat/parse-args-integration");
    compile_sources(
        &entry,
        &mut |from, name, _| internal(from, name),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "const { parseArgs } = require('internal/util/parse_args/parse_args'); module.exports = parseArgs({ args: ['--safe'], options: { safe: { type: 'boolean' } } }).values.safe;".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));

    assert_eq!(
        source(Path::new("node:internal/util/parse_args/parse_args.js"))
            .unwrap()
            .kind,
        ModuleKind::CommonJs
    );
}

#[test]
fn util_mime_and_diff_lazy_sources_compile() {
    let entry = file_path("nativeboat/util-lazy-integration");
    compile_sources(
        &entry,
        &mut |from, name, _| internal(from, name),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "const { MIMEType } = require('internal/mime'); const { diff } = require('internal/util/diff'); const mime = new MIMEType('text/plain; charset=utf-8'); module.exports = [mime.essence, mime.params.get('charset'), diff('ab', 'ac').length];".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
}

#[test]
fn public_assert_is_a_library_module() {
    let entry = file_path("nativeboat/assert-integration");
    let module = compile_sources(
        &entry,
        &mut |from, name, _| internal(from, name),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "'use strict'; const assert = require('assert'); assert(true); assert.strictEqual(1, 1); assert.deepEqual({value: 1}, {value: 1});".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert!(!module.native_dependencies().contains("assert"));
    assert_eq!(builtin("assert"), Some(PathBuf::from("node:assert")));
    let assert = source(Path::new("node:assert")).unwrap();
    assert!(matches!(assert.kind, ModuleKind::Native));
    assert_eq!(assert.text, "node:assert");
}

#[test]
fn constants_and_worker_identity_resolve_to_native_modules() {
    let entry = file_path("nativeboat/process-builtins-integration");
    let module = compile_sources(
        &entry,
        &mut |from, name, _| {
            builtin(name).or_else(|| internal(from, name))
        },
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "const constants = require('constants'); const { isatty } = require('tty'); const { threadId, isMainThread } = require('worker_threads'); module.exports = [constants.O_RDONLY, isatty(2), threadId, isMainThread];".into(),
                    kind: ModuleKind::CommonJs,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));

    assert_eq!(builtin("constants"), Some(PathBuf::from("constants")));
    assert_eq!(builtin("tty"), Some(PathBuf::from("tty")));
    assert_eq!(
        builtin("worker_threads"),
        Some(PathBuf::from("worker_threads"))
    );
    assert!(module.native_dependencies().contains("constants"));
    assert!(module.native_dependencies().contains("tty"));
    assert!(module.native_dependencies().contains("worker_threads"));
    assert_eq!(source(Path::new("constants")).unwrap().text, "constants");
    assert_eq!(source(Path::new("tty")).unwrap().text, "tty");
    assert_eq!(
        source(Path::new("worker_threads")).unwrap().text,
        "worker_threads"
    );
}

#[test]
fn native_keyboard_keymap_import_links_the_no_engine_platform_module() {
    let entry = file_path("nativeboat/native-keymap-integration");
    let module = compile_sources(
        &entry,
        &mut |from, name, _| builtin(name).or_else(|| internal(from, name)),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "import { getKeyMap, getCurrentKeyboardLayout, onDidChangeKeyboardLayout, isISOKeyboard } from 'native-keymap'; export { getKeyMap, getCurrentKeyboardLayout, onDidChangeKeyboardLayout, isISOKeyboard };".into(),
                    kind: ModuleKind::EsModule,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));

    assert_eq!(
        builtin("native-keymap"),
        Some(PathBuf::from("native-keymap"))
    );
    assert!(module.native_dependencies().contains("native-keymap"));
    assert!(
        !module
            .native_dependencies()
            .iter()
            .any(|name| name.starts_with("node:"))
    );
    assert_eq!(
        source(Path::new("native-keymap")).unwrap().text,
        "native-keymap"
    );
}

#[test]
fn native_keytar_import_links_the_engine_free_credentials_module() {
    let entry = file_path("nativeboat/keytar-integration");
    let module = compile_sources(
        &entry,
        &mut |from, name, _| builtin(name).or_else(|| internal(from, name)),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "import keytar from 'keytar'; export default keytar;".into(),
                    kind: ModuleKind::EsModule,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));

    assert_eq!(builtin("keytar"), Some(PathBuf::from("keytar")));
    assert!(module.native_dependencies().contains("keytar"));
    assert!(module.native_dependencies().iter().all(|dependency| {
        !["node", "electron", "v8", "boa"]
            .iter()
            .any(|forbidden| dependency.eq_ignore_ascii_case(forbidden))
    }));
}

#[test]
fn ced_import_links_the_packaged_no_engine_encoding_detector() {
    let entry = file_path("nativeboat/ced-integration");
    let module = compile_sources(
        &entry,
        &mut |from, name, _| builtin(name).or_else(|| internal(from, name)),
        &mut |path| {
            if path == entry {
                Some(ModuleSource {
                    text: "import ced from 'ced'; export default ced;".into(),
                    kind: ModuleKind::EsModule,
                })
            } else {
                source(path)
            }
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));

    assert_eq!(builtin("ced"), Some(PathBuf::from("ced")));
    assert!(module.native_dependencies().contains("ced"));
    assert!(module.native_dependencies().iter().all(|dependency| {
        !["node", "electron", "v8", "boa"]
            .iter()
            .any(|forbidden| dependency.eq_ignore_ascii_case(forbidden))
    }));
}

#[test]
fn public_builtins_map_to_library_sources() {
    assert_eq!(builtin("assert"), Some(PathBuf::from("node:assert")));
    assert_eq!(builtin("path"), Some(PathBuf::from("node:path")));
    assert_eq!(builtin("util"), Some(PathBuf::from("node:util")));
    assert_eq!(builtin("stream"), Some(PathBuf::from("node:stream")));
    assert_eq!(
        builtin("stream/promises"),
        Some(PathBuf::from("node:stream/promises"))
    );
    assert_eq!(
        builtin("querystring"),
        Some(PathBuf::from("node:querystring"))
    );
    assert_eq!(
        builtin("path/win32"),
        Some(PathBuf::from("node:path/win32"))
    );
    let placeholder = source(Path::new("node:path/win32")).unwrap();
    assert!(matches!(placeholder.kind, ModuleKind::Native));
    assert_eq!(placeholder.text, "node:path/win32");
    assert_eq!(builtin("internal/util"), None);
    assert_eq!(builtin("fs"), Some(PathBuf::from("node:fs")));
    assert_eq!(
        builtin("fs/promises"),
        Some(PathBuf::from("node:fs/promises"))
    );
    assert_eq!(builtin("url"), Some(PathBuf::from("node:url")));
    assert!(matches!(
        source(Path::new("node:util.js")).unwrap().kind,
        ModuleKind::CommonJs
    ));
    assert!(matches!(
        source(Path::new("node:stream.js")).unwrap().kind,
        ModuleKind::CommonJs
    ));
    assert!(matches!(
        source(Path::new("node:stream/promises.js")).unwrap().kind,
        ModuleKind::CommonJs
    ));
    assert_eq!(builtin("binding/url"), None);
}

#[test]
fn library_requests_stay_inside_the_library() {
    let library = Path::new("node:path.js");
    assert_eq!(
        internal(library, "internal/util"),
        Some(PathBuf::from("node:internal/util.js"))
    );
    assert_eq!(
        internal(library, "internal/errors"),
        Some(PathBuf::from("node:internal/errors"))
    );
    assert_eq!(
        internal(library, "path"),
        Some(PathBuf::from("node:path.js"))
    );
    // Internal modules that are not bundled are native placeholders.
    assert_eq!(
        internal(library, "internal/missing"),
        Some(PathBuf::from("node:internal/missing"))
    );
    // Modules the runtime provides natively are placeholders too.
    assert_eq!(internal(library, "tty"), Some(PathBuf::from("node:tty")));
    let placeholder = source(Path::new("node:tty")).unwrap();
    assert!(matches!(placeholder.kind, ModuleKind::Native));
    assert_eq!(internal(Path::new("C:/app/main.js"), "internal/util"), None);
}

#[test]
fn library_sources_receive_their_wrapper_bindings_on_the_directive_line() {
    let path = source(Path::new("node:path.js")).unwrap();
    assert!(matches!(path.kind, ModuleKind::CommonJs));
    assert!(
        path.text
            .contains("'use strict'; const primordials = require('internal/primordials');")
    );
    assert_eq!(
        path.text.lines().count(),
        include_str!("../../node/lib/path.js").lines().count()
    );
    let buffer = source(Path::new("node:buffer.js")).unwrap().text;
    assert!(buffer.contains(
        "'use strict'; const primordials = require('internal/primordials'); const internalBinding = require('internal/binding');"
    ));
    let stream_destroy = source(Path::new("node:internal/streams/destroy.js")).unwrap();
    assert!(matches!(stream_destroy.kind, ModuleKind::CommonJs));
    assert!(stream_destroy.text.contains("function destroy(err, cb)"));
    for name in ["internal/primordials", "internal/constants"] {
        let text = source(Path::new(&format!("node:{name}.js"))).unwrap().text;
        assert!(!text.contains("require('internal/primordials')"), "{name}");
    }
    let native = source(Path::new("node:internal\\errors")).unwrap();
    assert!(matches!(native.kind, ModuleKind::Native));
    assert_eq!(native.text, "internal/errors");
    let missing = source(Path::new("node:internal/missing")).unwrap();
    assert!(matches!(missing.kind, ModuleKind::Native));
    assert!(source(Path::new("node:internal/missing.js")).is_none());
    assert!(source(Path::new("node:path/missing.js")).is_none());
    assert!(source(Path::new("C:/app/main.js")).is_none());
}

fn compiled(text: &str) -> DynamicModule {
    let text = text.to_owned();
    nativeboat_compiler::compile_sources(
        Path::new("main.js"),
        &mut |_, name, _| builtin(name),
        &mut |path| {
            source(path).or_else(|| {
                Some(ModuleSource {
                    text: text.clone(),
                    kind: ModuleKind::EsModule,
                })
            })
        },
    )
    .unwrap()
}

#[test]
fn the_library_dispatches_public_modules_by_name() {
    let entry = source(Path::new("node:nativeboat/index.js")).unwrap();
    assert!(matches!(entry.kind, ModuleKind::CommonJs));
    assert!(
        entry
            .text
            .contains("case 'path/posix': return require('path/posix');")
    );
    assert!(entry.text.contains("case 'fs': return require('fs');"));
    assert!(
        entry
            .text
            .contains("internalBinding('buffer').setBufferPrototype(buffer.Buffer.prototype);")
    );
    assert!(
        entry
            .text
            .contains("case 'internal/fs/dir': return require('internal/fs/dir');")
    );
    assert!(
        entry
            .text
            .contains("case 'internal/fs/promises': return require('internal/fs/promises');")
    );
    assert!(!entry.text.contains("case 'nativeboat/URLSearchParams':"));
    assert!(builtin("nativeboat/URLSearchParams").is_none());
    assert!(builtin("internal/url/search_params").is_none());
    let mut events_only = std::collections::BTreeSet::new();
    events_only.insert("events".to_owned());
    let library = library(&events_only);
    assert!(library.starts_with("#![no_std]"));
    // Only the selected modules are dispatched, so others are not linked.
    assert!(library.contains("node:events.js") && !library.contains("node:zlib.js"));
    assert!(library.contains("pub fn require(realm: &mut Realm, name: &str)"));
    assert!(is_used(&compiled(
        "import path from 'path'; export const sep = path.sep;"
    )));
    assert!(is_used(&compiled("export const kind = typeof Buffer;")));
    assert!(is_used(&compiled(
        "export const params = new URLSearchParams('a=1');"
    )));
    assert!(is_used(&compiled(
        "export const constructor = globalThis.URL;"
    )));
    assert!(is_used(&compiled(
        "export const constructor = global['URLSearchParams'];"
    )));
    assert!(!is_used(&compiled(
        "export function f(globalThis) { return globalThis.URL; }"
    )));
    assert!(!is_used(&compiled("export const value = 1;")));
}

#[test]
fn applications_compile_beside_the_library() {
    // This binary also compiles the Node.js library, so it runs the
    // compiler's block, loop and nested finally lowering for coverage.
    let module = compiled(
        "export function f() { { let a = 1; () => a; } for (let i = 0; i < 1; i++) { () => i; }          try { try { return 1; } finally { 2; } } finally { 3; } }",
    );
    assert!(selected(std::iter::once(&module)).is_none());
}

fn is_used(module: &DynamicModule) -> bool {
    selected(std::iter::once(module)).is_some()
}

#[test]
fn programs_link_the_library_modules_they_use() {
    let path = selected(std::iter::once(&compiled(
        "import path from 'path'; export const sep = path.sep;",
    )))
    .unwrap();
    assert!(path.contains("path") && path.contains("events") && path.contains("buffer"));
    assert!(!path.contains("zlib"));
    let electron = compile_sources(
        Path::new("main.mjs"),
        &mut |_, request, _| (request == "electron").then(|| PathBuf::from("electron")),
        &mut |path| {
            Some(if path == Path::new("main.mjs") {
                ModuleSource {
                    text: "import { app } from 'electron'; app.on('ready', () => {});".to_owned(),
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
    .unwrap();
    let electron = selected(std::iter::once(&electron)).unwrap();
    // `buffer` loads its Blob and File modules lazily, by name; `events`
    // requires `async_hooks`, which the dispatcher then names too.
    assert_eq!(
        electron.into_iter().collect::<Vec<_>>(),
        [
            "async_hooks",
            "buffer",
            "events",
            "internal/blob",
            "internal/file"
        ]
    );
    let files = selected(std::iter::once(&compiled(
        "import fs from 'fs'; export const exists = fs.existsSync;",
    )))
    .unwrap();
    assert!(files.contains("fs") && files.contains("internal/fs/dir"));
    // The message port globals, and worker_threads, whose port classes
    // are the library's, link the module of the ports.
    let ports = selected(std::iter::once(&compiled(
        "export const port = new MessageChannel().port1;",
    )))
    .unwrap();
    assert!(ports.contains("internal/worker/io"));
    let threads = compile_sources(
        Path::new("main.mjs"),
        &mut |_, request, _| builtin(request),
        &mut |path| {
            Some(if path == Path::new("main.mjs") {
                ModuleSource {
                    text: "import { isMainThread } from 'worker_threads'; export const main = isMainThread;"
                        .to_owned(),
                    kind: ModuleKind::EsModule,
                }
            } else {
                ModuleSource {
                    text: "worker_threads".to_owned(),
                    kind: ModuleKind::Native,
                }
            })
        },
    )
    .unwrap();
    assert!(
        selected(std::iter::once(&threads))
            .unwrap()
            .contains("internal/worker/io")
    );
    // `crypto` is a library module over the native binding: it links the
    // buffers its hashes and ciphers take and the decoder of their output
    // (their streams are internal modules the graph links).
    let crypto = selected(std::iter::once(&compiled(
        "import { createHash } from 'crypto'; export const hash = createHash('sha1');",
    )))
    .unwrap();
    for name in ["crypto", "string_decoder", "buffer"] {
        assert!(crypto.contains(name), "{name}: {crypto:?}");
    }
    assert!(!NATIVE.contains(&"crypto"));
}

/// Every native addon binary the build resolves to the runtime is one the
/// runtime's module loader provides, and the runtime lists the same ones.
#[test]
fn runtime_provides_each_native_binding_the_build_resolves() {
    let modules = include_str!("../../../nativeboat-native/src/modules.rs");
    for binding in nativeboat_bundler::resolve::NATIVE_BINDINGS {
        assert!(modules.contains(&format!("{binding:?} =>")), "{binding}");
        assert!(modules.contains(&format!("    {binding:?},")), "{binding}");
        assert_eq!(builtin(binding), Some(PathBuf::from(binding)));
    }
    for adapter in nativeboat_bundler::resolve::NATIVE_ADAPTERS {
        assert!(modules.contains(&format!("{adapter:?} =>")), "{adapter}");
        assert_eq!(builtin(adapter), Some(PathBuf::from(adapter)));
    }
}
