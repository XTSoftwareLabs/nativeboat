//! The catalog of features the analyzer reports.

use serde::{Deserialize, Serialize};

mod gradients;

/// How well the native target supports a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Support {
    /// Works like in Electron.
    Supported,
    /// Works with limitations described in the feature note.
    Partial,
    /// Not available in the native target.
    Unsupported,
}

impl Support {
    /// The label used in reports.
    pub fn label(self) -> &'static str {
        match self {
            Support::Supported => "Supported",
            Support::Partial => "Partial",
            Support::Unsupported => "Unsupported",
        }
    }

    /// The contribution to the compatibility score.
    pub fn weight(self) -> f64 {
        match self {
            Support::Supported => 1.0,
            Support::Partial => 0.5,
            Support::Unsupported => 0.0,
        }
    }
}

/// A feature that can appear in a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    /// Stable identifier such as `electron:dialog`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Support level.
    pub support: Support,
    /// What is missing for partial or unsupported features.
    pub note: String,
}

fn feature(id: &str, name: &str, support: Support, note: &str) -> Feature {
    Feature {
        id: id.to_owned(),
        name: name.to_owned(),
        support,
        note: note.to_owned(),
    }
}

use Support::{Partial, Supported, Unsupported};

const REACT_NOTE: &str = "runs on the native DOM; validated on small applications only";

/// Frameworks and tools recognized from `package.json`.
pub fn framework(package: &str) -> Option<Feature> {
    let (name, support, note) = match package {
        "react" => ("React", Partial, REACT_NOTE),
        "react-dom" => ("React DOM", Partial, REACT_NOTE),
        "typescript" => ("TypeScript", Supported, ""),
        "vite" => ("Vite", Supported, ""),
        "electron-vite" => ("electron-vite", Supported, ""),
        "vite-plugin-electron" => ("vite-plugin-electron", Supported, ""),
        "@electron-forge/plugin-vite" => ("Electron Forge (Vite)", Supported, ""),
        "webpack" => (
            "Webpack",
            Partial,
            "sources are compiled directly; custom loaders are not applied",
        ),
        "vue" => ("Vue", Supported, ""),
        "svelte" => (
            "Svelte",
            Partial,
            "runs on the native DOM; not yet validated",
        ),
        "preact" => (
            "Preact",
            Partial,
            "runs on the native DOM; not yet validated",
        ),
        "solid-js" => (
            "Solid",
            Partial,
            "runs on the native DOM; not yet validated",
        ),
        "jquery" => (
            "jQuery",
            Partial,
            "runs on the native DOM; layout queries are approximate",
        ),
        "@angular/core" => (
            "Angular",
            Unsupported,
            "zone.js and the Angular compiler are not supported",
        ),
        "sass" | "node-sass" | "less" | "stylus" => (
            "CSS preprocessors",
            Unsupported,
            "only plain CSS and CSS modules are compiled",
        ),
        _ => return None,
    };
    Some(feature(
        &format!("framework:{package}"),
        name,
        support,
        note,
    ))
}

/// Packages known to contain native Node.js addons.
pub const NATIVE_PACKAGES: &[&str] = &[
    "better-sqlite3",
    "sqlite3",
    "node-pty",
    "serialport",
    "@serialport/bindings-cpp",
    "sharp",
    "robotjs",
    "keytar",
    "ced",
    "clip-filepaths",
    "native-keymap",
    "ffi-napi",
    "ref-napi",
    "usb",
    "node-hid",
    "canvas",
    "bcrypt",
    "argon2",
    "leveldown",
    "nodegit",
    "realm",
];

/// A dependency that ships a native addon.
pub fn native_package(package: &str) -> Feature {
    let (support, note) = match package {
        "ced" => (
            Supported,
            "Buffer encoding detection uses Google CED with the package's options and encoding labels; no addon is loaded",
        ),
        "clip-filepaths" => (
            Supported,
            "file lists are read from and written to the Windows clipboard as Explorer copies them; no addon is loaded",
        ),
        "keytar" => (
            Supported,
            "the five password APIs use Windows Credential Manager with keytar's storage format and Promise results; no addon is loaded",
        ),
        "native-keymap" => (
            Supported,
            "physical key maps, current layout metadata and TSF layout-change notifications use Windows; isISOKeyboard preserves the Windows result; no addon is loaded",
        ),
        "@vscode/windows-registry" => (
            Supported,
            "string and DWORD values are read from the Windows registry; no addon is loaded",
        ),
        "@vscode/deviceid" => (
            Supported,
            "the device id is read from and stored in the current user's registry as the addon stores it; no addon is loaded",
        ),
        "@vscode/windows-mutex" => (
            Supported,
            "named mutexes are Windows mutexes; no addon is loaded",
        ),
        "@vscode/policy-watcher" => (
            Supported,
            "a product's Group Policy values and their changes come from the Windows registry; no addon is loaded",
        ),
        "@vscode/windows-process-tree" => (
            Supported,
            "process lists, memory and CPU use come from Windows snapshots, at most 1024 processes as the addon lists; no addon is loaded",
        ),
        "@vscode/windows-ca-certs" | "@vscode/os-proxy-resolver" => (
            Supported,
            "system certificates and proxy settings come from Windows; the package is replaced and no addon is loaded",
        ),
        "@vscode/spdlog" => (
            Partial,
            "rotating loggers write files as spdlog 1.12 does; other logger types write to standard output; no addon is loaded",
        ),
        "@vscode/sqlite3" => (
            Partial,
            "databases use the SQLite that Windows includes (3.38 or later); where it is missing or older, the runtime's own engine runs tables, indexes, queries, inserts, updates, deletes, transactions, pragmas, VACUUM and some ALTER TABLE forms, but not all of SQL; no addon is loaded",
        ),
        "@parcel/watcher" => (
            Partial,
            "directory trees are watched with ReadDirectoryChangesW and events coalesce as parcel's do; snapshots (writeSnapshot, getEventsSince) are not written; no addon is loaded",
        ),
        "node-pty" => (
            Supported,
            "terminals run in Windows pseudoconsoles as node-pty's Windows terminal does; the package is replaced and no addon is loaded",
        ),
        "native-is-elevated" => (
            Supported,
            "elevation comes from the process token; no addon is loaded",
        ),
        "windows-foreground-love" => (
            Supported,
            "other processes are allowed the foreground with AllowSetForegroundWindow, as the addon does; no addon is loaded",
        ),
        "@vscode/fs-copyfile" => (
            Supported,
            "on Windows the package copies with Node's fs.copyFile and loads its addon only on macOS",
        ),
        "@vscode/native-watchdog" => (
            Supported,
            "the process exits when its parent ends, as the addon checks; no addon is loaded",
        ),
        _ => (
            Unsupported,
            "native Node.js addons cannot be loaded by the native runtime",
        ),
    };
    feature(
        &format!("native:{package}"),
        &format!("Native module {package}"),
        support,
        note,
    )
}

/// Electron modules by export name.
pub fn electron(api: &str) -> Feature {
    let (support, note) = match api {
        "app" | "BrowserWindow" | "ipcMain" | "ipcRenderer" | "contextBridge" | "dialog"
        | "Menu" | "MenuItem" | "clipboard" | "shell" | "nativeTheme" | "screen" | "webUtils"
        | "Tray" | "globalShortcut" | "safeStorage" | "powerSaveBlocker" | "powerMonitor"
        | "systemPreferences" | "nativeImage" | "MessageChannelMain" => (Supported, ""),
        "desktopCapturer" => (
            Partial,
            "displays are listed as screen sources with empty thumbnails; windows are not listed",
        ),
        "contentTracing" => (
            Partial,
            "recordings record no events: stopRecording writes an empty trace",
        ),
        "utilityProcess" => (
            Partial,
            "fork starts modules the build finds by their literal paths; execArgv, serviceName and inherited stdio are not used",
        ),
        "Notification" => (
            Partial,
            "shown as notification area balloons, which Windows shows as banners; actions, replies and urgency are not available",
        ),
        "protocol" => (
            Partial,
            "registerSchemesAsPrivileged, registerFileProtocol, registerBufferProtocol, registerStringProtocol, handle, unhandle, isProtocolHandled and unregisterProtocol are available; a handler's answer must come at once or from a promise job, not after I/O or a timer",
        ),
        "webContents" => (
            Partial,
            "getAllWebContents, fromId and getFocusedWebContents are available; contents are not found by frame or DevTools target",
        ),
        "webFrame" => (
            Partial,
            "zoom and visual zoom limits are available (pages are not pinch-zoomed); other methods, such as spell checking, are not",
        ),
        "net" => (Partial, "fetch and request are available"),
        "crashReporter" => (
            Partial,
            "start writes local crash dumps; reports are not uploaded",
        ),
        "session" => (
            Partial,
            "the default session's setProxy and resolveProxy, and fromPartition, are available",
        ),
        "autoUpdater" => (
            Partial,
            "behaves as in applications Squirrel.Windows did not install: checks report that Squirrel is missing; electron-updater updates native builds",
        ),
        _ => (Unsupported, "not available in the native runtime"),
    };
    let name = format!("Electron {api}");
    feature(&format!("electron:{api}"), &name, support, note)
}

/// What the runtime provides of the Electron modules it supports in part,
/// judged member by member.
const PARTIAL_ELECTRON: &[(&str, &[&str])] = &[
    ("crashReporter", &["start"]),
    (
        "protocol",
        &[
            "registerSchemesAsPrivileged",
            "registerFileProtocol",
            "registerBufferProtocol",
            "registerStringProtocol",
            "handle",
            "unhandle",
            "isProtocolHandled",
            "unregisterProtocol",
        ],
    ),
    ("net", &["fetch", "request", "isOnline", "online"]),
    ("utilityProcess", &["fork"]),
    (
        "webContents",
        &["fromId", "getAllWebContents", "getFocusedWebContents"],
    ),
    (
        "session",
        &[
            "defaultSession.resolveProxy",
            "defaultSession.setProxy",
            "defaultSession.protocol",
            "defaultSession.fetch",
            "defaultSession.getUserAgent",
            "defaultSession.isPersistent",
            "defaultSession.getStoragePath",
            "defaultSession.registerPreloadScript",
            "defaultSession.unregisterPreloadScript",
            "defaultSession.getPreloadScripts",
            "defaultSession.setPreloads",
            "defaultSession.getPreloads",
            "fromPartition",
            "fromPath",
        ],
    ),
    (
        "webFrame",
        &[
            "getZoomFactor",
            "getZoomLevel",
            "setVisualZoomLevelLimits",
            "setZoomFactor",
            "setZoomLevel",
        ],
    ),
];

/// Members of the default session that are accepted without doing what
/// Electron does: native pages ask no permissions and load without
/// Chromium's network service.
const INERT_SESSION: &[(&str, &str)] = &[
    (
        "setPermissionRequestHandler",
        "the handler is not called: native pages ask no permissions",
    ),
    (
        "setPermissionCheckHandler",
        "the handler is not called: native pages ask no permissions",
    ),
    (
        "setDisplayMediaRequestHandler",
        "the handler is not called: pages do not capture the screen",
    ),
    (
        "setDevicePermissionHandler",
        "the handler is not called: native pages ask no permissions",
    ),
    ("setCertificateVerifyProc", "the procedure is not called"),
    (
        "webRequest",
        "listeners are kept but no request reaches them",
    ),
    ("setCodeCachePath", "native builds have no code cache"),
    (
        "setSpellCheckerDictionaryDownloadURL",
        "the Windows spell checker's dictionaries are used",
    ),
    ("clearData", "resolves; there is no browsing data to clear"),
    ("clearCache", "resolves; there is no HTTP cache to clear"),
    ("clearStorageData", "resolves without clearing page storage"),
    ("setUserAgent", "the user agent stays the native one"),
];

/// Whether the members of an Electron module are judged one by one.
pub fn is_partial_electron(api: &str) -> bool {
    PARTIAL_ELECTRON.iter().any(|(partial, _)| *partial == api)
}

/// A member of an Electron module, such as `webFrame.setZoomFactor`, or of
/// the default session, such as `defaultSession.resolveProxy`.
pub fn electron_member(api: &str, member: &str) -> Feature {
    match PARTIAL_ELECTRON.iter().find(|(partial, _)| *partial == api) {
        None => electron(api),
        // The default session used other than through its members.
        Some(_) if api == "session" && member == "defaultSession" => electron(api),
        Some((_, members)) if members.contains(&member) => feature(
            &format!("electron:{api}"),
            &format!("Electron {api}"),
            Supported,
            "",
        ),
        Some(_)
            if api == "session"
                && let Some((_, note)) = INERT_SESSION
                    .iter()
                    .find(|(name, _)| member.strip_prefix("defaultSession.") == Some(*name)) =>
        {
            feature(
                &format!("electron:{api}.{member}"),
                &format!("Electron {api}.{member}"),
                Partial,
                note,
            )
        }
        Some(_) => feature(
            &format!("electron:{api}.{member}"),
            &format!("Electron {api}.{member}"),
            Unsupported,
            "not available in the native runtime",
        ),
    }
}

/// The members of a window's `webContents` that work as in Electron: its
/// own methods, and an event emitter's.
const WEB_CONTENTS: &[&str] = &[
    "id",
    "session",
    "send",
    // Message ports move to the renderer, as `ipcRenderer` event ports.
    "postMessage",
    "isDestroyed",
    "getUserAgent",
    "getZoomFactor",
    "setZoomFactor",
    "getZoomLevel",
    "setZoomLevel",
    "undo",
    "redo",
    "cut",
    "copy",
    "paste",
    "pasteAndMatchStyle",
    "delete",
    "selectAll",
    "insertText",
    "replaceMisspelling",
    "isDevToolsOpened",
    "printToPDF",
    "print",
    "setIgnoreMenuShortcuts",
    "getOSProcessId",
    "getProcessId",
    "getTitle",
    "isFocused",
    "isCrashed",
    "isOffscreen",
    "isLoadingMainFrame",
    "isWaitingForResponse",
    "isDevToolsFocused",
    "loadURL",
    "loadFile",
    "close",
    "setWebRTCIPHandlingPolicy",
    "getWebRTCIPHandlingPolicy",
    // macOS's dictionary lookup: Electron on Windows does nothing either.
    "showDefinitionForSelection",
    "on",
    "once",
    "off",
    "addListener",
    "removeListener",
    "removeAllListeners",
    "prependListener",
    "prependOnceListener",
    "emit",
    "listeners",
    "rawListeners",
    "listenerCount",
    "eventNames",
    "setMaxListeners",
    "getMaxListeners",
];

/// The members of a window's `webContents` that are accepted without doing
/// what Electron does.
const INERT_WEB_CONTENTS: &[(&str, &str)] = &[
    (
        "executeJavaScript",
        "resolves without running the script in the page",
    ),
    ("openDevTools", "developer tools are not available"),
    ("closeDevTools", "developer tools are not available"),
    ("toggleDevTools", "developer tools are not available"),
    ("reload", "does not reload the page"),
    ("focus", "does not move the keyboard focus"),
    ("setWindowOpenHandler", "the handler is not called"),
    ("setVisualZoomLevelLimits", "pages are not pinch-zoomed"),
    ("setAudioMuted", "does not mute the page"),
    ("isLoading", "always false"),
    ("getURL", "always an empty string"),
    (
        "setUserAgent",
        "getUserAgent reports it; pages keep the native navigator.userAgent",
    ),
    (
        "findInPage",
        "returns a request id; pages are not searched, so no found-in-page event follows",
    ),
    ("stopFindInPage", "pages are not searched"),
    ("reloadIgnoringCache", "does not reload the page"),
    ("setBackgroundThrottling", "pages are not throttled"),
    ("startDrag", "files are not dragged out of the window"),
    ("copyImageAt", "does not copy the image"),
    ("beginFrameSubscription", "frames are not delivered"),
    ("endFrameSubscription", "frames are not delivered"),
    ("inspectElement", "developer tools are not available"),
];

/// A member of a window's `webContents`, such as `win.webContents.send`.
/// Those that work are part of `BrowserWindow`.
pub fn web_contents_member(member: &str) -> Feature {
    if WEB_CONTENTS.contains(&member) {
        return electron("BrowserWindow");
    }
    let (support, note) = match INERT_WEB_CONTENTS.iter().find(|(name, _)| *name == member) {
        Some((_, note)) => (Partial, *note),
        None => (Unsupported, "not available in the native runtime"),
    };
    feature(
        &format!("electron:webContents.{member}"),
        &format!("Electron webContents.{member}"),
        support,
        note,
    )
}

/// Node.js built-in modules, used as a whole.
pub fn node(module: &str) -> Feature {
    let base = module.split('/').next().unwrap_or(module);
    let (support, note) = match base {
        "fs"
        | "path"
        | "os"
        | "events"
        | "buffer"
        | "util"
        | "url"
        | "process"
        | "assert"
        | "querystring"
        | "string_decoder"
        | "timers"
        | "readline"
        | "stream"
        | "http"
        | "diagnostics_channel"
        | "console" => (Supported, ""),
        "perf_hooks" => (
            Partial,
            "performance marks and measures are available; observers receive no entries and histograms no samples",
        ),
        "vm" => (
            Partial,
            "contexts can be made; evaluating or compiling code throws, as a native build has no JavaScript engine",
        ),
        "inspector" => (
            Partial,
            "no debugger attaches: url() is undefined, and opening the inspector or connecting a session throws",
        ),
        "v8" => (
            Partial,
            "heap statistics are zero and flags have no effect; snapshots and serialization are not available",
        ),
        "dns" => (
            Partial,
            "lookup is available; DNS queries (resolve, reverse, lookupService) fail with ENOTIMP",
        ),
        "sqlite" => (
            Unsupported,
            "the module loads; opening a database is not available",
        ),
        "crypto" => (
            Partial,
            "MD5, SHA-1 and SHA-2 digests, HMACs, PBKDF2, HKDF, AES ciphers, random values, secret keys and X.509 certificates are available; public-key cryptography, Diffie-Hellman and scrypt are not",
        ),
        "child_process" | "module" | "async_hooks" | "http2" => {
            (Partial, "a subset of the module is available")
        }
        "net" => (
            Partial,
            "TCP sockets and servers, and Windows named pipes, are available",
        ),
        "https" => (
            Partial,
            "requests and streamed responses are available; servers and custom agents are not",
        ),
        "tls" => (
            Partial,
            "constants and classes are available; secure connections and servers are not",
        ),
        "zlib" => (Partial, "deflate and gzip are available; brotli is not"),
        "constants" => (
            Partial,
            "filesystem and Windows OS constants are available; OpenSSL constants are not",
        ),
        "tty" => (
            Partial,
            "isatty queries standard Windows handles; terminal streams are not available",
        ),
        "worker_threads" => (
            Partial,
            "the main thread's identity and message ports within the process are available; worker threads are not",
        ),
        _ => (Unsupported, "not available in the native runtime"),
    };
    feature(
        &format!("node:{module}"),
        &format!("Node {module}"),
        support,
        note,
    )
}

/// What the runtime provides of the Node.js modules it supports in part.
const PARTIAL_EXPORTS: &[(&str, &[&str])] = &[
    (
        "child_process",
        &[
            "exec",
            "execFile",
            "execFileSync",
            "execSync",
            "spawn",
            "spawnSync",
        ],
    ),
    (
        "net",
        &[
            "Server",
            "Socket",
            "Stream",
            "connect",
            "createConnection",
            "createServer",
            "isIP",
            "isIPv4",
            "isIPv6",
            "getDefaultAutoSelectFamily",
            "setDefaultAutoSelectFamily",
            "getDefaultAutoSelectFamilyAttemptTimeout",
            "setDefaultAutoSelectFamilyAttemptTimeout",
        ],
    ),
    ("https", &["Agent", "globalAgent", "get", "request"]),
    (
        "crypto",
        &[
            "constants",
            "createCipheriv",
            "createDecipheriv",
            "createHash",
            "createHmac",
            "createSecretKey",
            "generateKey",
            "generateKeySync",
            "getCipherInfo",
            "getCiphers",
            "getCurves",
            "getFips",
            "getHashes",
            "getRandomValues",
            "hash",
            "hkdf",
            "hkdfSync",
            "pbkdf2",
            "pbkdf2Sync",
            "pseudoRandomBytes",
            "randomBytes",
            "randomFill",
            "randomFillSync",
            "randomInt",
            "randomUUID",
            "secureHeapUsed",
            "subtle",
            "timingSafeEqual",
            "webcrypto",
            "Cipheriv",
            "Decipheriv",
            "Hash",
            "Hmac",
            "KeyObject",
            "X509Certificate",
        ],
    ),
    (
        "module",
        &[
            "createRequire",
            "isBuiltin",
            "builtinModules",
            "registerHooks",
            "constants",
            "enableCompileCache",
            "flushCompileCache",
            "getCompileCacheDir",
            "globalPaths",
            "_nodeModulePaths",
            "_resolveLookupPaths",
        ],
    ),
    ("async_hooks", &["AsyncLocalStorage", "AsyncResource"]),
    (
        "worker_threads",
        &[
            "BroadcastChannel",
            "MessageChannel",
            "MessagePort",
            "isMainThread",
            "parentPort",
            "receiveMessageOnPort",
            "threadId",
            "workerData",
        ],
    ),
    (
        "http2",
        &[
            "constants",
            "getDefaultSettings",
            "getPackedSettings",
            "getUnpackedSettings",
            "sensitiveHeaders",
            "Http2ServerRequest",
            "Http2ServerResponse",
        ],
    ),
];

/// One export of a Node.js module. The exports of modules the runtime
/// supports in part are judged one by one: what it provides is supported.
pub fn node_export(module: &str, name: &str) -> Feature {
    let provided = if module == "zlib" {
        Some(!name.to_ascii_lowercase().contains("brotli"))
    } else {
        PARTIAL_EXPORTS
            .iter()
            .find(|(partial, _)| *partial == module)
            .map(|(_, names)| names.contains(&name))
    };
    if (module, name) == ("child_process", "fork") {
        return feature(
            "node:child_process.fork",
            "Node child_process.fork",
            Partial,
            "starts modules the build finds by their literal paths in the program's own executable; handles are not sent, and execPath and execArgv are not used",
        );
    }
    match provided {
        None => node(module),
        Some(true) => feature(
            &format!("node:{module}"),
            &format!("Node {module}"),
            Supported,
            "",
        ),
        Some(false) => {
            let note = match (module, name) {
                ("zlib", _) => "brotli compression is not available",
                _ => "not available in the native runtime",
            };
            feature(
                &format!("node:{module}.{name}"),
                &format!("Node {module}.{name}"),
                Unsupported,
                note,
            )
        }
    }
}

/// Whether the runtime supports only part of a Node.js module, so that its
/// exports are judged one by one.
pub fn is_partial_module(module: &str) -> bool {
    module == "zlib"
        || PARTIAL_EXPORTS
            .iter()
            .any(|(partial, _)| *partial == module)
}

/// Web platform APIs used from JavaScript.
pub fn web(api: &str) -> Option<Feature> {
    let (name, support, note) = match api {
        "webgl" => ("WebGL", Unsupported, "requires a GPU browser stack"),
        "worker" => (
            "Web Workers",
            Partial,
            "dedicated workers run in realms of their own on the page's thread; a blob script runs only when it declares, assigns, calls and imports; no SharedWorker",
        ),
        "service-worker" => (
            "Service Workers",
            Partial,
            "registered workers run in realms of their own on the page's thread and install and activate at once; their fetch event answers the fetch() calls, images and linked style sheets of the pages they control, not their scripts, fonts or navigations; caches live in memory",
        ),
        "websocket" => (
            "WebSocket",
            Partial,
            "ws: and wss: clients over WinHTTP where Node's globals are: the main process, utility processes, forked children and pages with Node integration; other pages do not have it yet",
        ),
        "web-crypto" => (
            "Web Crypto",
            Partial,
            "SHA digests, HMAC, AES-GCM, AES-CBC and AES-CTR with secret keys in raw and JWK formats, PBKDF2 and HKDF; RSA, elliptic curves and key wrapping are not available",
        ),
        "webassembly" => (
            "WebAssembly",
            Partial,
            "modules run in the runtime's interpreter; no SIMD, threads or exception handling",
        ),
        "fetch" => (
            "fetch",
            Partial,
            "local files and HTTP(S) requests; no CORS or caching",
        ),
        "xhr" => ("XMLHttpRequest", Partial, "asynchronous requests only"),
        "indexeddb" => (
            "IndexedDB",
            Unsupported,
            "use localStorage or the file system",
        ),
        "web-audio" => ("Web Audio", Unsupported, "no audio stack"),
        "media-devices" => (
            "Media devices",
            Unsupported,
            "no camera or microphone access",
        ),
        "local-storage" => ("Web Storage", Supported, ""),
        "drag-drop" => ("HTML drag and drop", Supported, ""),
        "drag-image" => (
            "Custom drag images",
            Partial,
            "drags show the platform's image, not the one setDragImage sets",
        ),
        "resize-observer" => ("ResizeObserver", Supported, ""),
        "mutation-observer" => ("MutationObserver", Supported, ""),
        "intersection-observer" => ("IntersectionObserver", Supported, ""),
        "shadow-dom" => (
            "Shadow DOM",
            Partial,
            "shadow trees render with their own style sheets, :host, :host-context() and ::slotted(), and retarget events and focus; no adoptedStyleSheets, ::part(), slotchange or manual slot assignment",
        ),
        "custom-elements" => (
            "Custom elements",
            Partial,
            "autonomous and customized built-in elements upgrade and get their lifecycle callbacks; no form-associated elements, ElementInternals or adoptedCallback",
        ),
        "window-open" => (
            "window.open",
            Partial,
            "opens a new native window or the default browser",
        ),
        "clipboard-api" => (
            "Clipboard API",
            Partial,
            "only readText and writeText are available",
        ),
        "clipboard-text" => ("Clipboard API text", Supported, ""),
        "web-notifications" => (
            "Web notifications",
            Partial,
            "shown as notification area balloons with their icons; show, click, close and error events fire; actions are not available",
        ),
        _ => return None,
    };
    Some(feature(&format!("web:{api}"), name, support, note))
}

/// Canvas 2D, with what a use relies on that is not drawn as in a browser.
pub fn canvas_2d(gap: Option<&str>) -> Feature {
    match gap {
        None => feature("web:canvas-2d", "Canvas", Supported, ""),
        Some(note) => feature("web:canvas-2d", "Canvas", Partial, note),
    }
}

/// The commands `document.execCommand` runs, in lowercase.
const EDITING_COMMANDS: &[&str] = &[
    "copy",
    "cut",
    "delete",
    "forwarddelete",
    "insertlinebreak",
    "insertparagraph",
    "inserttext",
    "paste",
    "redo",
    "selectall",
    "undo",
];

/// `document.execCommand`, judged by its command when that is a literal.
pub fn exec_command(command: Option<&str>) -> Feature {
    const AVAILABLE: &str =
        "clipboard, undo, redo, selectAll and text insertion and deletion commands are available";
    let (support, note) = match command.map(str::to_ascii_lowercase) {
        Some(command) if EDITING_COMMANDS.contains(&command.as_str()) => (Supported, String::new()),
        Some(command) => (
            Partial,
            format!("the {command} command is not available; {AVAILABLE}"),
        ),
        None => (Partial, AVAILABLE.to_owned()),
    };
    feature("web:exec-command", "document.execCommand", support, &note)
}

/// HTML elements, grouped into reported features.
pub fn element(tag: &str) -> Option<Feature> {
    let lowercase = tag.to_ascii_lowercase();
    let (id, name, support, note) = match lowercase.as_str() {
        "iframe" | "frame" | "frameset" => (
            "iframe",
            "iframe",
            Unsupported,
            "embedded browsing contexts need a browser engine",
        ),
        "webview" => (
            "webview",
            "webview tag",
            Unsupported,
            "embedded browsing contexts need a browser engine",
        ),
        "video" | "audio" | "source" | "track" => (
            "media",
            "Audio and video",
            Partial,
            "videos show their poster or first frame, and media loads its duration and loading events, through the codecs Windows has; playback, controls and captions are not available",
        ),
        "object" | "embed" | "applet" => (
            "plugin",
            "Plugins (object/embed)",
            Unsupported,
            "browser plugins are not supported",
        ),
        "canvas" => ("canvas", "Canvas element", Supported, ""),
        "svg" => ("svg", "SVG", Supported, ""),
        // SVG content the renderer does not draw.
        "lineargradient" | "radialgradient" | "pattern" | "clippath" | "mask" | "filter"
        | "image" | "textpath" | "animate" | "animatemotion" | "animatetransform" | "set" => (
            "svg",
            "SVG",
            Partial,
            "shapes, paths, text, markers and use references are drawn; gradients, patterns, clipping paths, masks, filters, images, text on paths and SVG animations are not",
        ),
        name if name.starts_with("fe") && name.len() > 2 => (
            "svg",
            "SVG",
            Partial,
            "shapes, paths, text, markers and use references are drawn; gradients, patterns, clipping paths, masks, filters, images, text on paths and SVG animations are not",
        ),
        "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "colgroup" | "col"
        | "caption" => ("table", "Tables", Supported, ""),
        "details" | "summary" => ("details", "details and summary", Supported, ""),
        "dialog" => (
            "dialog",
            "dialog",
            Partial,
            "show() and close() work; showModal() opens the dialog in place, without the top layer, a backdrop or making the rest of the page inert",
        ),
        "input" | "button" | "textarea" | "select" | "option" | "optgroup" | "label" | "form"
        | "fieldset" | "legend" => ("controls", "Form controls", Supported, ""),
        _ => return None,
    };
    Some(feature(&format!("html:{id}"), name, support, note))
}

/// The generic HTML feature reported once for ordinary markup.
pub fn html_basics() -> Feature {
    feature("html:basics", "HTML elements", Supported, "")
}

/// CSS features detected from property names.
pub fn css_property(name: &str, value: &str) -> Option<Feature> {
    let value = value.to_ascii_lowercase();
    let (id, display, support, note) = match name {
        "display" if value.contains("flex") => ("flexbox", "Flexbox", Supported, ""),
        "display" if value.contains("grid") => ("grid", "CSS Grid", Supported, ""),
        "display" if value.contains("table") => ("table", "CSS tables", Supported, ""),
        "display" if value.contains("contents") => ("contents", "display: contents", Supported, ""),
        "position" if value.contains("sticky") => ("sticky", "position: sticky", Supported, ""),
        "clip" => ("clip", "CSS rectangular clipping", Supported, ""),
        "appearance" | "-webkit-appearance"
            if !matches!(
                value.trim().trim_end_matches("!important").trim(),
                "auto" | "none"
            ) =>
        {
            (
                "appearance",
                "CSS appearance",
                Partial,
                "none takes form controls' native look away and auto keeps it; other values keep the native look",
            )
        }
        "background-clip" if value.contains("text") => (
            "background-clip-text",
            "CSS background-clip: text",
            Partial,
            "backgrounds clipped to text are not painted; the text keeps its own color",
        ),
        name if name.starts_with("scroll-margin") || name.starts_with("scroll-padding") => {
            ("scroll-spacing", "CSS scroll spacing", Supported, "")
        }
        "float" | "clear" => ("float", "Floats", Supported, ""),
        "transform" | "translate" | "rotate" | "scale" | "transform-origin" => {
            ("transform", "CSS transforms", Supported, "")
        }
        "transition"
        | "transition-property"
        | "transition-duration"
        | "transition-timing-function"
        | "transition-delay" => ("transition", "CSS transitions", Supported, ""),
        "animation"
        | "animation-name"
        | "animation-duration"
        | "animation-iteration-count"
        | "animation-timing-function"
        | "animation-delay"
        | "animation-fill-mode"
        | "animation-direction"
        | "animation-play-state" => ("animation", "CSS animations", Supported, ""),
        "filter" if value.contains("url(") => (
            "filter-url",
            "SVG filter references",
            Unsupported,
            "filters defined in SVG documents are not applied",
        ),
        "filter" => (
            "filter",
            "CSS filters",
            Supported,
            "filter functions, including blurs and drop shadows; native controls inside filtered elements are drawn unfiltered",
        ),
        "scrollbar-width" | "scrollbar-color" => {
            ("scrollbars", "Scrollbar colors and parts", Supported, "")
        }
        "backdrop-filter" | "-webkit-backdrop-filter" if value.trim() == "none" => return None,
        "backdrop-filter" | "-webkit-backdrop-filter" => (
            "backdrop-filter",
            "CSS backdrop filters",
            Supported,
            "what lies behind the element is filtered, other scrolling surfaces and native controls included",
        ),
        "mix-blend-mode" if value.trim() == "normal" => return None,
        "mix-blend-mode" => (
            "blend",
            "CSS blend modes",
            Unsupported,
            "blend modes other than normal are not supported",
        ),
        "clip-path" | "-webkit-clip-path" if value.trim() == "none" => return None,
        // Shapes, boxes and the document's own <clipPath> elements clip
        // exactly; clip paths in other documents do not.
        "clip-path" | "-webkit-clip-path" => {
            let value = value.trim().to_ascii_lowercase();
            let elsewhere = value.starts_with("url(")
                && !value
                    .trim_start_matches("url(")
                    .trim_start_matches(['"', '\''])
                    .starts_with('#');
            if elsewhere {
                (
                    "clip-path",
                    "CSS clip-path",
                    Partial,
                    "url() references to clip paths in other documents do not clip",
                )
            } else {
                ("clip-path", "CSS clip-path", Supported, "")
            }
        }
        "mask" | "mask-image" | "-webkit-mask" | "-webkit-mask-image" if value.trim() == "none" => {
            return None;
        }
        // Images and gradients mask by their alpha; SVG <mask> elements do
        // not mask.
        "mask" | "mask-image" | "-webkit-mask" | "-webkit-mask-image" => {
            let value = value.trim().to_ascii_lowercase().replace(['"', '\''], "");
            if value.contains("url(#") {
                (
                    "mask",
                    "CSS masks",
                    Partial,
                    "references to SVG mask elements do not mask",
                )
            } else {
                ("mask", "CSS masks", Supported, "")
            }
        }
        "columns" | "column-count" | "column-width" => (
            "columns",
            "Multi-column layout",
            Unsupported,
            "not supported",
        ),
        "box-shadow" => (
            "shadow",
            "Shadows",
            Supported,
            "Gaussian box shadows with inset, spread and rounded corners",
        ),
        "text-shadow" if value.trim() == "none" => return None,
        "text-shadow" => ("shadow", "Shadows", Partial, "text shadows are not drawn"),
        // The native target never anchors scrolling, as `none` asks.
        "overflow-anchor" if value.trim() == "none" => return None,
        "overflow-anchor" => (
            "overflow-anchor",
            "Scroll anchoring",
            Partial,
            "scroll positions are not adjusted when content above changes",
        ),
        "background-image" | "background" if value.contains("gradient") => (
            "gradient",
            "CSS gradients",
            if gradients::supported(&value).is_some() {
                Supported
            } else {
                Partial
            },
            "opaque single-layer linear gradients support angles, single-side directions and increasing percentage stops, including stops outside 0–100%; corner directions, alpha stops, length stops, hard stops, repeating/radial gradients and multiple layers remain partial",
        ),
        _ => return None,
    };
    Some(feature(&format!("css:{id}"), display, support, note))
}

/// Background images in scrollbar parts, which are not drawn.
pub fn scrollbar_images() -> Feature {
    feature(
        "css:scrollbars",
        "Scrollbar colors and parts",
        Partial,
        "colors, thickness, track, thumb and corner boxes and their hover and active states are drawn; background images are not",
    )
}

/// CSS at-rules.
pub fn css_at_rule(name: &str) -> Option<Feature> {
    let (id, display, support, note) = match name {
        "media" => ("media", "Media queries", Supported, ""),
        "supports" => ("supports", "@supports", Supported, ""),
        "keyframes" | "-webkit-keyframes" => ("animation", "CSS animations", Supported, ""),
        "font-face" => (
            "font-face",
            "Web fonts",
            Partial,
            "fonts in the application's files and data URLs load; fonts at http and https addresses and fonts scripts add with FontFace do not",
        ),
        "container" => (
            "container",
            "Container queries",
            Partial,
            "size queries (width, height, inline-size, block-size, aspect-ratio, orientation) apply to elements; style() queries never match, and container-type does not contain the container's size",
        ),
        "layer" => ("layer", "Cascade layers", Supported, ""),
        "property" => ("property", "@property", Supported, ""),
        "import" => ("import", "@import", Supported, ""),
        _ => return None,
    };
    Some(feature(&format!("css:{id}"), display, support, note))
}

/// Keyframes that set properties animations do not interpolate.
pub fn keyframes_with_static(properties: &[&str]) -> Feature {
    feature(
        "css:animation",
        "CSS animations",
        Partial,
        &format!(
            "{} keep their value; the properties browsers interpolate are animated",
            properties.join(", ")
        ),
    )
}

/// CSS custom properties.
pub fn css_variables() -> Feature {
    feature("css:variables", "CSS custom properties", Supported, "")
}

/// Selectors using pseudo-classes the native target cannot evaluate.
pub fn css_selector(pseudo: &str) -> Feature {
    feature(
        &format!("css:selector{pseudo}"),
        &format!("CSS selector {pseudo}"),
        Unsupported,
        "the pseudo-class never matches",
    )
}

/// Properties the style engine does not know at all.
pub fn css_unknown_property(name: &str) -> Feature {
    feature(
        &format!("css:property:{name}"),
        &format!("CSS property {name}"),
        Partial,
        "the property is ignored",
    )
}

/// Decorators where the project compiles TypeScript's
/// `experimentalDecorators`.
pub fn legacy_decorators() -> Feature {
    feature("language:decorators", "Decorators", Supported, "")
}

/// Language features of the application code.
pub fn language(id: &str) -> Option<Feature> {
    let (name, support, note) = match id {
        "decorators" => (
            "Decorators",
            Unsupported,
            "only TypeScript's experimentalDecorators are compiled",
        ),
        "top-level-await" => (
            "Top-level await",
            Partial,
            "a module that awaits runs as an async function: dynamic import() waits for it, but a module that imports it statically runs on without waiting",
        ),
        "dynamic-require" => (
            "Dynamic require",
            Partial,
            "only statically known modules are bundled",
        ),
        "remote-script" => (
            "Remote scripts",
            Unsupported,
            "scripts must be part of the application",
        ),
        _ => return None,
    };
    Some(feature(&format!("language:{id}"), name, support, note))
}

#[cfg(test)]
mod tests;
