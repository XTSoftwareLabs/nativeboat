//! Node.js library modules compiled from Node's own sources. The bundled
//! Node.js v22.15.0 sources under `node/lib` are compiled once into the
//! `nativeboat_node` library: a dispatching module requires each public
//! module, linked like any CommonJS graph, and `internal/...` requests stay
//! inside the library (`internal/errors` is a native runtime module).
//! Applications import built-in modules such as `path` as `node:` native
//! modules, which the runtime takes from that library. Sources are served
//! from virtual `node:` paths, so no source file is written or read.

use nativeboat_compiler::{DynamicModule, ModuleKind, ModuleSource, compile_sources};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The prefix of every library module path.
const PREFIX: &str = "node:";

/// Library sources by module name.
const FILES: &[(&str, crate::Text)] = &[
    ("assert", nativeboat_conceal::text!("../node/lib/assert.js")),
    ("crypto", nativeboat_conceal::text!("../node/lib/crypto.js")),
    (
        "internal/crypto/x509",
        nativeboat_conceal::text!("../node/lib/internal/crypto/x509.js"),
    ),
    (
        "console",
        nativeboat_conceal::text!("../node/lib/console.js"),
    ),
    (
        "diagnostics_channel",
        nativeboat_conceal::text!("../node/lib/diagnostics_channel.js"),
    ),
    ("dns", nativeboat_conceal::text!("../node/lib/dns.js")),
    (
        "dns/promises",
        nativeboat_conceal::text!("../node/lib/dns/promises.js"),
    ),
    ("module", nativeboat_conceal::text!("../node/lib/module.js")),
    (
        "perf_hooks",
        nativeboat_conceal::text!("../node/lib/perf_hooks.js"),
    ),
    ("sqlite", nativeboat_conceal::text!("../node/lib/sqlite.js")),
    (
        "util/types",
        nativeboat_conceal::text!("../node/lib/util/types.js"),
    ),
    ("v8", nativeboat_conceal::text!("../node/lib/v8.js")),
    ("vm", nativeboat_conceal::text!("../node/lib/vm.js")),
    (
        "inspector",
        nativeboat_conceal::text!("../node/lib/inspector.js"),
    ),
    (
        "async_hooks",
        nativeboat_conceal::text!("../node/lib/async_hooks.js"),
    ),
    ("http", nativeboat_conceal::text!("../node/lib/http.js")),
    ("http2", nativeboat_conceal::text!("../node/lib/http2.js")),
    ("https", nativeboat_conceal::text!("../node/lib/https.js")),
    (
        "internal/electron/net",
        nativeboat_conceal::text!("../node/lib/internal/electron/net.js"),
    ),
    (
        "internal/electron/utility_process",
        nativeboat_conceal::text!("../node/lib/internal/electron/utility_process.js"),
    ),
    (
        "internal/electron/media",
        nativeboat_conceal::text!("../node/lib/internal/electron/media.js"),
    ),
    (
        "internal/using_ctx",
        nativeboat_conceal::text!("../node/lib/internal/using_ctx.js"),
    ),
    (
        "internal/net",
        nativeboat_conceal::text!("../node/lib/internal/net.js"),
    ),
    (
        "internal/readline/promises",
        nativeboat_conceal::text!("../node/lib/internal/readline/promises.js"),
    ),
    ("net", nativeboat_conceal::text!("../node/lib/net.js")),
    (
        "readline",
        nativeboat_conceal::text!("../node/lib/readline.js"),
    ),
    (
        "readline/promises",
        nativeboat_conceal::text!("../node/lib/readline/promises.js"),
    ),
    ("tls", nativeboat_conceal::text!("../node/lib/tls.js")),
    (
        "internal/abort_controller",
        nativeboat_conceal::text!("../node/lib/internal/abort_controller.js"),
    ),
    (
        "internal/fs/dir",
        nativeboat_conceal::text!("../node/lib/internal/fs/dir.js"),
    ),
    (
        "internal/fs/watchers",
        nativeboat_conceal::text!("../node/lib/internal/fs/watchers.js"),
    ),
    (
        "internal/fs/recursive_watch",
        nativeboat_conceal::text!("../node/lib/internal/fs/recursive_watch.js"),
    ),
    (
        "internal/fs/rimraf",
        nativeboat_conceal::text!("../node/lib/internal/fs/rimraf.js"),
    ),
    (
        "internal/fs/streams",
        nativeboat_conceal::text!("../node/lib/internal/fs/streams.js"),
    ),
    (
        "internal/encoding",
        nativeboat_conceal::text!("../node/lib/internal/encoding.js"),
    ),
    (
        "internal/file",
        nativeboat_conceal::text!("../node/lib/internal/file.js"),
    ),
    (
        "internal/webidl",
        nativeboat_conceal::text!("../node/lib/internal/webidl.js"),
    ),
    (
        "internal/webstreams/queuingstrategies",
        nativeboat_conceal::text!("../node/lib/internal/webstreams/queuingstrategies.js"),
    ),
    (
        "internal/webstreams/adapters",
        nativeboat_conceal::text!("../node/lib/internal/webstreams/adapters.js"),
    ),
    (
        "internal/webstreams/compression",
        nativeboat_conceal::text!("../node/lib/internal/webstreams/compression.js"),
    ),
    (
        "internal/webstreams/encoding",
        nativeboat_conceal::text!("../node/lib/internal/webstreams/encoding.js"),
    ),
    (
        "internal/webstreams/readablestream",
        nativeboat_conceal::text!("../node/lib/internal/webstreams/readablestream.js"),
    ),
    (
        "internal/webstreams/transformstream",
        nativeboat_conceal::text!("../node/lib/internal/webstreams/transformstream.js"),
    ),
    (
        "internal/webstreams/util",
        nativeboat_conceal::text!("../node/lib/internal/webstreams/util.js"),
    ),
    (
        "internal/webstreams/writablestream",
        nativeboat_conceal::text!("../node/lib/internal/webstreams/writablestream.js"),
    ),
    (
        "internal/fetch",
        nativeboat_conceal::text!("../node/lib/internal/fetch.js"),
    ),
    (
        "internal/navigator",
        nativeboat_conceal::text!("../node/lib/internal/navigator.js"),
    ),
    (
        "internal/websocket",
        nativeboat_conceal::text!("../node/lib/internal/websocket.js"),
    ),
    (
        "internal/webassembly",
        nativeboat_conceal::text!("../node/lib/internal/webassembly.js"),
    ),
    (
        "internal/worker/io",
        nativeboat_conceal::text!("../node/lib/internal/worker/io.js"),
    ),
    (
        "stream/web",
        nativeboat_conceal::text!("../node/lib/stream/web.js"),
    ),
    (
        "internal/streams/utils",
        nativeboat_conceal::text!("../node/lib/internal/streams/utils.js"),
    ),
    (
        "internal/streams/destroy",
        nativeboat_conceal::text!("../node/lib/internal/streams/destroy.js"),
    ),
    (
        "internal/streams/add-abort-signal",
        nativeboat_conceal::text!("../node/lib/internal/streams/add-abort-signal.js"),
    ),
    (
        "internal/streams/compose",
        nativeboat_conceal::text!("../node/lib/internal/streams/compose.js"),
    ),
    (
        "internal/streams/duplex",
        nativeboat_conceal::text!("../node/lib/internal/streams/duplex.js"),
    ),
    (
        "internal/streams/duplexify",
        nativeboat_conceal::text!("../node/lib/internal/streams/duplexify.js"),
    ),
    (
        "internal/streams/duplexpair",
        nativeboat_conceal::text!("../node/lib/internal/streams/duplexpair.js"),
    ),
    (
        "internal/streams/end-of-stream",
        nativeboat_conceal::text!("../node/lib/internal/streams/end-of-stream.js"),
    ),
    (
        "internal/streams/from",
        nativeboat_conceal::text!("../node/lib/internal/streams/from.js"),
    ),
    (
        "internal/streams/lazy_transform",
        nativeboat_conceal::text!("../node/lib/internal/streams/lazy_transform.js"),
    ),
    (
        "internal/streams/legacy",
        nativeboat_conceal::text!("../node/lib/internal/streams/legacy.js"),
    ),
    (
        "internal/streams/operators",
        nativeboat_conceal::text!("../node/lib/internal/streams/operators.js"),
    ),
    (
        "internal/streams/passthrough",
        nativeboat_conceal::text!("../node/lib/internal/streams/passthrough.js"),
    ),
    (
        "internal/streams/pipeline",
        nativeboat_conceal::text!("../node/lib/internal/streams/pipeline.js"),
    ),
    (
        "internal/streams/readable",
        nativeboat_conceal::text!("../node/lib/internal/streams/readable.js"),
    ),
    (
        "internal/streams/state",
        nativeboat_conceal::text!("../node/lib/internal/streams/state.js"),
    ),
    (
        "internal/streams/transform",
        nativeboat_conceal::text!("../node/lib/internal/streams/transform.js"),
    ),
    (
        "internal/streams/writable",
        nativeboat_conceal::text!("../node/lib/internal/streams/writable.js"),
    ),
    ("stream", nativeboat_conceal::text!("../node/lib/stream.js")),
    ("util", nativeboat_conceal::text!("../node/lib/util.js")),
    (
        "stream/promises",
        nativeboat_conceal::text!("../node/lib/stream/promises.js"),
    ),
    (
        "string_decoder",
        nativeboat_conceal::text!("../node/lib/string_decoder.js"),
    ),
    ("fs", nativeboat_conceal::text!("../node/lib/fs.js")),
    (
        "fs/promises",
        nativeboat_conceal::text!("../node/lib/fs/promises.js"),
    ),
    (
        "internal/fs/promises",
        nativeboat_conceal::text!("../node/lib/internal/fs/promises.js"),
    ),
    (
        "internal/readline/interface",
        nativeboat_conceal::text!("../node/lib/internal/readline/interface.js"),
    ),
    (
        "internal/readline/callbacks",
        nativeboat_conceal::text!("../node/lib/internal/readline/callbacks.js"),
    ),
    (
        "internal/readline/utils",
        nativeboat_conceal::text!("../node/lib/internal/readline/utils.js"),
    ),
    (
        "internal/readline/emitKeypressEvents",
        nativeboat_conceal::text!("../node/lib/internal/readline/emitKeypressEvents.js"),
    ),
    (
        "internal/fs/read/context",
        nativeboat_conceal::text!("../node/lib/internal/fs/read/context.js"),
    ),
    (
        "internal/process/permission",
        nativeboat_conceal::text!("../node/lib/internal/process/permission.js"),
    ),
    (
        "internal/blob",
        nativeboat_conceal::text!("../node/lib/internal/blob.js"),
    ),
    ("buffer", nativeboat_conceal::text!("../node/lib/buffer.js")),
    ("events", nativeboat_conceal::text!("../node/lib/events.js")),
    ("os", nativeboat_conceal::text!("../node/lib/os.js")),
    ("path", nativeboat_conceal::text!("../node/lib/path.js")),
    (
        "path/posix",
        nativeboat_conceal::text!("../node/lib/path/posix.js"),
    ),
    (
        "path/win32",
        nativeboat_conceal::text!("../node/lib/path/win32.js"),
    ),
    (
        "process",
        nativeboat_conceal::text!("../node/lib/process.js"),
    ),
    (
        "querystring",
        nativeboat_conceal::text!("../node/lib/querystring.js"),
    ),
    ("url", nativeboat_conceal::text!("../node/lib/url.js")),
    ("timers", nativeboat_conceal::text!("../node/lib/timers.js")),
    (
        "timers/promises",
        nativeboat_conceal::text!("../node/lib/timers/promises.js"),
    ),
    ("zlib", nativeboat_conceal::text!("../node/lib/zlib.js")),
    (
        "internal/url",
        nativeboat_conceal::text!("../node/lib/internal/url.js"),
    ),
    (
        "internal/assert",
        nativeboat_conceal::text!("../node/lib/internal/assert.js"),
    ),
    (
        "internal/fs/utils",
        nativeboat_conceal::text!("../node/lib/internal/fs/utils.js"),
    ),
    (
        "internal/buffer",
        nativeboat_conceal::text!("../node/lib/internal/buffer.js"),
    ),
    (
        "internal/constants",
        nativeboat_conceal::text!("../node/lib/internal/constants.js"),
    ),
    (
        "internal/deps/minimatch/index",
        nativeboat_conceal::text!("../node/lib/internal/deps/minimatch/index.js"),
    ),
    (
        "internal/event_target",
        nativeboat_conceal::text!("../node/lib/internal/event_target.js"),
    ),
    (
        "internal/events/abort_listener",
        nativeboat_conceal::text!("../node/lib/internal/events/abort_listener.js"),
    ),
    (
        "internal/per_context/domexception",
        nativeboat_conceal::text!("../node/lib/internal/per_context/domexception.js"),
    ),
    (
        "internal/process/per_thread",
        nativeboat_conceal::text!("../node/lib/internal/process/per_thread.js"),
    ),
    (
        "internal/worker/js_transferable",
        nativeboat_conceal::text!("../node/lib/internal/worker/js_transferable.js"),
    ),
    (
        "internal/primordials",
        nativeboat_conceal::text!("../node/lib/internal/primordials.js"),
    ),
    (
        "internal/querystring",
        nativeboat_conceal::text!("../node/lib/internal/querystring.js"),
    ),
    (
        "internal/util",
        nativeboat_conceal::text!("../node/lib/internal/util.js"),
    ),
    (
        "internal/util/types",
        nativeboat_conceal::text!("../node/lib/internal/util/types.js"),
    ),
    (
        "internal/util/parse_args/parse_args",
        nativeboat_conceal::text!("../node/lib/internal/util/parse_args/parse_args.js"),
    ),
    (
        "internal/mime",
        nativeboat_conceal::text!("../node/lib/internal/mime.js"),
    ),
    (
        "internal/util/diff",
        nativeboat_conceal::text!("../node/lib/internal/util/diff.js"),
    ),
    (
        "internal/util/debuglog",
        nativeboat_conceal::text!("../node/lib/internal/util/debuglog.js"),
    ),
    (
        "internal/util/comparisons",
        nativeboat_conceal::text!("../node/lib/internal/util/comparisons.js"),
    ),
    (
        "internal/validators",
        nativeboat_conceal::text!("../node/lib/internal/validators.js"),
    ),
];

/// Library modules the native runtime provides.
const NATIVE: &[&str] = &[
    "internal/async_hooks",
    "internal/binding",
    "internal/errors",
    "internal/options",
    "internal/process/task_queues",
    "internal/util/inspect",
    // These built-ins are small native Windows facilities used by the
    // unchanged Node sources (rather than JavaScript library modules).
    "constants",
    "tty",
    "worker_threads",
];

fn file_path(name: &str) -> PathBuf {
    PathBuf::from(format!("{PREFIX}{name}.js"))
}

/// Whether a module name is a public module of the library.
fn public(name: &str) -> bool {
    matches!(
        name,
        "assert"
            | "async_hooks"
            | "buffer"
            | "console"
            | "crypto"
            | "diagnostics_channel"
            | "dns"
            | "dns/promises"
            | "events"
            | "fs"
            | "fs/promises"
            | "http"
            | "http2"
            | "https"
            | "module"
            | "net"
            | "os"
            | "path"
            | "path/posix"
            | "path/win32"
            | "perf_hooks"
            | "process"
            | "querystring"
            | "readline"
            | "readline/promises"
            | "sqlite"
            | "stream"
            | "stream/promises"
            | "stream/web"
            | "string_decoder"
            | "timers"
            | "timers/promises"
            | "tls"
            | "util"
            | "util/types"
            | "url"
            | "v8"
            | "vm"
            | "inspector"
            | "zlib"
    )
}

/// An application's request for a public built-in module: a native module
/// the runtime takes from the precompiled library.
pub(crate) fn builtin(name: &str) -> Option<PathBuf> {
    if public(name) {
        Some(PathBuf::from(format!("{PREFIX}{name}")))
    } else if matches!(
        name,
        "child_process" | "constants" | "tty" | "worker_threads" | "electron"
    ) || nativeboat_bundler::resolve::NATIVE_ADAPTERS.contains(&name)
        || nativeboat_bundler::resolve::NATIVE_BINDINGS.contains(&name)
    {
        Some(PathBuf::from(name))
    } else {
        None
    }
}

/// The library's dispatching module.
const ENTRY: &str = "nativeboat/index";

/// The dispatching module: its default export returns the exports of a
/// selected module by name. Like Node's bootstrap, it gives the buffer
/// binding `Buffer.prototype` once `buffer` loads, and reads `NODE_DEBUG`
/// for `debuglog` before any module logs.
fn entry(selected: &BTreeSet<String>) -> String {
    let mut text = String::from(
        "'use strict';\n\
         const internalBinding = require('internal/binding');\n\
         let bufferPrototype = false;\n\
         let debugEnv = false;\n\
         module.exports = (name) => {\n  \
         if (!debugEnv) {\n    \
         debugEnv = true;\n    \
         require('internal/util/debuglog').initializeDebugEnv(globalThis.process?.env?.NODE_DEBUG);\n  \
         }\n  switch (name) {\n",
    );
    for (name, _) in FILES.iter().filter(|(name, _)| selected.contains(*name)) {
        if *name == "buffer" {
            text.push_str(
                "    case 'buffer': {\n      const buffer = require('buffer');\n      \
                 if (!bufferPrototype) {\n        \
                 internalBinding('buffer').setBufferPrototype(buffer.Buffer.prototype);\n        \
                 bufferPrototype = true;\n      }\n      return buffer;\n    }\n",
            );
        } else {
            text.push_str(&format!("    case '{name}': return require('{name}');\n"));
        }
    }
    text.push_str("  }\n};\n");
    text
}

/// The `nativeboat_node` crate for a program: the selected modules, the
/// modules they require, and its `require`.
pub(crate) fn library(selected: &BTreeSet<String>) -> String {
    let module = compile_sources(
        &file_path(ENTRY),
        &mut |from, name, _| internal(from, name),
        &mut |path| {
            if path == file_path(ENTRY) {
                return Some(ModuleSource {
                    text: entry(selected),
                    kind: ModuleKind::CommonJs,
                });
            }
            source(path)
        },
    )
    .expect("the bundled Node.js library compiles");
    format!(
        "#![no_std]\nextern crate alloc;\n{}\n\
         /// The exports of a Node.js library module.\n\
         pub fn require(realm: &mut Realm, name: &str) -> ::nativeboat_native::Completion {{\n\
         ::nativeboat_native::modules::library(realm, module, name)\n}}\n",
        module.rust_source()
    )
}

/// The modules a library module requires by a string literal, as
/// `require('path')`.
fn statically_required(text: &str) -> impl Iterator<Item = &str> {
    text.split("require(").skip(1).filter_map(|call| {
        let quote = call.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        let rest = &call[1..];
        let (name, _) = rest.split_once(quote)?;
        Some(name)
    })
}

/// The modules a library module loads on first use by name.
fn lazily_loaded(text: &str) -> impl Iterator<Item = &str> {
    text.split("defineLazyProperties(")
        .skip(1)
        .filter_map(|call| {
            let (_, rest) = call.split_once('\'')?;
            let (name, _) = rest.split_once('\'')?;
            Some(name)
        })
}

/// The library modules a program's contexts need, or `None` when they need
/// none. Electron's `app`, windows and web contents are Node event emitters,
/// so programs using Electron need `events`; Electron's `net.request` goes
/// over the `http` and `https` modules, so programs using Electron that
/// link either get it. The runtime itself loads
/// `events` for `process` and `buffer` to make Buffers, so any program that
/// links the library gets both. Only the selected modules and the modules
/// they require are compiled and linked.
pub(crate) fn selected<'a>(
    modules: impl Iterator<Item = &'a DynamicModule>,
) -> Option<BTreeSet<String>> {
    let mut selected = BTreeSet::new();
    let mut electron = false;
    for module in modules {
        for name in module.native_dependencies() {
            if let Some(name) = name.strip_prefix(PREFIX) {
                selected.insert(name.to_owned());
            } else if name == "electron" {
                electron = true;
                selected.insert("events".to_owned());
            } else if name == "worker_threads" {
                // Its message port classes are the library's.
                selected.insert("internal/worker/io".to_owned());
            }
        }
    }
    if selected.is_empty() {
        return None;
    }
    if electron && (selected.contains("http") || selected.contains("https")) {
        selected.insert("internal/electron/net".to_owned());
    }
    selected.extend(["events", "buffer"].map(str::to_owned));
    // `defineLazyProperties(target, 'internal/fs/dir', [...])` loads a
    // module by name on first use, through the dispatcher: select the
    // modules a selected module loads so.
    let mut pending: Vec<String> = selected.iter().cloned().collect();
    while let Some(name) = pending.pop() {
        let Some((_, text)) = FILES.iter().find(|(file, _)| *file == name) else {
            continue;
        };
        for lazy in lazily_loaded(text()) {
            if selected.insert(lazy.to_owned()) {
                pending.push(lazy.to_owned());
            }
        }
        // Public modules a selected one requires are linked with it; the
        // dispatcher names them too, so a `require` made at run time (the
        // `module` library's `createRequire`) loads them by name.
        for required in statically_required(text()) {
            if public(required) && selected.insert(required.to_owned()) {
                pending.push(required.to_owned());
            }
        }
    }
    Some(selected)
}

/// A request made by a library module: other library modules, including
/// internal and native ones, by name.
pub(crate) fn internal(from: &Path, name: &str) -> Option<PathBuf> {
    if !from.to_str().is_some_and(|from| from.starts_with(PREFIX)) {
        return None;
    }
    if FILES.iter().any(|(file, _)| *file == name) {
        return Some(file_path(name));
    }
    // Modules that are not bundled are native: they report that they are
    // not compiled only if a lazy `require` reaches them.
    Some(PathBuf::from(format!("{PREFIX}{name}")))
}

/// The source of a library module path. Modules receive `primordials` and
/// `internalBinding` as Node's internal module wrapper supplies them, on
/// the line of the `'use strict'` directive so source positions are
/// unchanged.
pub(crate) fn source(path: &Path) -> Option<ModuleSource> {
    if let Some(name) = path.to_str()
        && (matches!(
            name,
            "child_process" | "constants" | "tty" | "worker_threads" | "electron"
        ) || nativeboat_bundler::resolve::NATIVE_ADAPTERS.contains(&name)
            || nativeboat_bundler::resolve::NATIVE_BINDINGS.contains(&name))
    {
        return Some(ModuleSource {
            text: name.to_owned(),
            kind: ModuleKind::Native,
        });
    }
    let name = path.to_str()?.strip_prefix(PREFIX)?.replace('\\', "/");
    if NATIVE.contains(&name.as_str()) || (!name.ends_with(".js") && !public(&name)) {
        return Some(ModuleSource {
            text: name,
            kind: ModuleKind::Native,
        });
    }
    if public(&name) {
        return Some(ModuleSource {
            text: format!("{PREFIX}{name}"),
            kind: ModuleKind::Native,
        });
    }
    let name = name.strip_suffix(".js")?;
    if name == ENTRY {
        return Some(ModuleSource {
            text: entry(&FILES.iter().map(|(name, _)| (*name).to_owned()).collect()),
            kind: ModuleKind::CommonJs,
        });
    }
    let text = FILES.iter().find(|(file, _)| *file == name)?.1();
    let mut wrapper = String::from("'use strict';");
    for (binding, module) in [
        ("primordials", "internal/primordials"),
        ("internalBinding", "internal/binding"),
    ] {
        if name != module && text.contains(binding) {
            wrapper.push_str(&format!(" const {binding} = require('{module}');"));
        }
    }
    let text = text.replacen("'use strict';", &wrapper, 1);
    Some(ModuleSource {
        text,
        kind: ModuleKind::CommonJs,
    })
}

#[cfg(test)]
mod tests;
