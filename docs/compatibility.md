# Compatibility

What NativeBoat 0.1.0-alpha.7 supports in an Electron application, in three
levels:

- **Supported**: works as in Electron.
- **Partial**: works with the limits given.
- **Not supported**: not available in a compiled application.

`nativeboat analyze` uses the same levels and shows where your project uses
each feature. If code that can run uses a feature that is not supported,
`nativeboat build` stops with an error that points to the source.

NativeBoat builds for Windows 10 version 1607 or later and Windows 11, on x64.
It does not build for macOS or Linux.

## Projects and tools

| Feature | Support | Notes |
| --- | --- | --- |
| Plain Electron, JavaScript and TypeScript (`tsc`) | Supported | |
| electron-vite, vite-plugin-electron, Vite | Supported | |
| Electron Forge with the Vite plugin | Supported | |
| electron-builder configuration | Supported | app metadata, `files`, `extraResources`, `extraFiles`, output folder, `publish`, `electronLanguages`; in `package.json`, YAML, JSON, JavaScript or TypeScript files |
| electron-updater | Supported | updates from the feed that a settings file given to `nativeboat build --config`, `nativeboat.publish` in `package.json` or electron-builder's `publish` names; each update downloads the whole installer |
| Windows installer (`nsis`) | Partial | installs for the current user with shortcuts, an uninstaller and update metadata (`latest.yml`); `artifactName` and the shortcut settings apply; `oneClick: false`, `perMachine`, `allowToChangeInstallationDirectory`, `license`, `include` and `script` are ignored with a warning |
| Plugin packages loaded by path (the `plugins` setting) | Partial | the main module of each package in the folders the setting names is built into the application and loads when the application requires or imports its path; packages added after the build cannot be loaded |
| npm, pnpm, Yarn, Bun | Supported | |
| Vue | Supported | |
| Tailwind CSS 4 (`@tailwindcss/vite`) | Supported | |
| React | Partial | runs on the native DOM; validated on small applications only |
| Webpack | Partial | sources are compiled directly; custom loaders are not applied |
| Svelte, Preact, Solid | Partial | runs on the native DOM; not yet validated |
| jQuery | Partial | layout queries are approximate |
| Angular | Not supported | zone.js and the Angular compiler are not supported |
| Sass, Less, Stylus | Not supported | only plain CSS and CSS modules are compiled |

## Language

| Feature | Support | Notes |
| --- | --- | --- |
| Modern JavaScript and TypeScript | Supported | |
| JSX and TSX | Supported | React's automatic runtime or the factory the TypeScript configuration names |
| TypeScript `experimentalDecorators` | Supported | |
| TypeScript enums, namespaces and `import x = require()` | Supported | |
| `using` and `await using` declarations | Supported | |
| Top-level `await` | Partial | a module that awaits runs as an async function: dynamic `import()` waits for it, but a module that imports it statically runs on without waiting |
| Dynamic `require` | Partial | only modules known at build time are included; others throw `MODULE_NOT_FOUND` when they run |
| `import()` with a specifier computed while the application runs | Partial | loads the modules the build finds from the values the program gives the specifier; other values fail to load |
| Function source text (`toString`) | Partial | functions whose text the application's code reads, with `toString`, `String()`, a template or by joining it to a string, give their source without TypeScript's types; other functions give no source text |
| Standard decorators | Not supported | only TypeScript's `experimentalDecorators` are compiled; class `accessor` fields are not compiled either |
| `eval` and `new Function` with source text | Not supported | code cannot be made from strings while the application runs |
| Scripts that pages build as text and insert as `<script>` elements | Partial | a script built from a template literal in the application's code that creates script elements runs; other script text does not |
| Remote scripts | Not supported | scripts must be part of the application |

## Electron APIs

| API | Support | Notes |
| --- | --- | --- |
| `app`, `BrowserWindow` | Supported | including transparent, always-on-top and taskbar-less windows, `autoHideMenuBar` and window icons |
| A window's `webContents` | Partial | messages and message ports, zoom, editing commands, printing, loading and events work; `executeJavaScript` resolves without running the script, `reload` does not reload, and developer tools and find in page are not available; members such as `capturePage` are not available |
| `ipcMain`, `ipcRenderer`, `contextBridge` | Supported | |
| `dialog`, `Menu`, `MenuItem` | Supported | |
| `clipboard`, `shell` | Supported | |
| `Tray`, `globalShortcut` | Supported | |
| `nativeTheme`, `screen`, `webUtils` | Supported | |
| `nativeImage` | Supported | |
| `webFrame` | Partial | zoom only (`get`/`setZoomFactor`, `get`/`setZoomLevel`, `setVisualZoomLevelLimits`, though pages are not pinch-zoomed); spell checking and other methods are not available |
| `Notification` | Partial | shown as notification area balloons, which Windows shows as banners; actions, replies and urgency are not available |
| `protocol` | Partial | `registerSchemesAsPrivileged`, `registerFileProtocol`, `registerBufferProtocol`, `registerStringProtocol`, `handle`, `unhandle`, `isProtocolHandled` and `unregisterProtocol`; pages load style sheets, images, fonts, media and `fetch` requests from handlers, but a handler must answer at once or from a promise, not after file or network work or a timer; other methods are not available |
| `net` | Partial | `net.fetch` and `net.request` only |
| `session` | Partial | `defaultSession`, `fromPartition` and `fromPath`, which give one shared session, with `setProxy`, `resolveProxy`, `protocol`, `fetch`, preload scripts and the storage path; permission handlers, `webRequest` and cache clearing are accepted without effect; `cookies` and other members are not available; the analyzer judges the default session by the members an application names |
| `utilityProcess` | Partial | `fork` starts modules the build finds by their literal paths, with `parentPort` and message ports; `execArgv`, `serviceName` and inherited standard streams are not used |
| `MessageChannelMain` | Supported | ports move to pages and utility processes |
| `desktopCapturer` | Partial | displays are listed as screen sources with empty thumbnails; windows are not listed |
| `contentTracing` | Partial | recordings record no events: `stopRecording` writes an empty trace |
| `crashReporter` | Partial | `start` writes local crash dumps; reports are not uploaded |
| `autoUpdater` | Partial | behaves as in applications Squirrel.Windows did not install: checks report that Squirrel is missing; the electron-updater package updates native builds |
| `safeStorage` | Supported | encrypts as Electron does on Windows, so text an Electron build of the application encrypted still decrypts |
| `powerSaveBlocker` | Supported | |
| `powerMonitor` | Supported | idle time and state, power source, and the sleep, wake, lock and power source events |
| `systemPreferences` | Supported | the methods and events Electron has on Windows: accent and system colors, animation settings and media access |
| The `webContents` module | Partial | `getAllWebContents`, `fromId` and `getFocusedWebContents`; contents are not found by frame or DevTools target |
| Any other module | Not supported | |

## Node.js modules

| Module | Support | Notes |
| --- | --- | --- |
| `fs`, `path`, `os`, `events`, `buffer`, `util`, `url` | Supported | |
| `process`, `assert`, `querystring`, `string_decoder` | Supported | |
| `timers`, `stream`, `stream/web`, `readline` | Supported | |
| `http`, `console`, `diagnostics_channel` | Supported | |
| `original-fs` | Supported | the same as `fs`, as compiled applications have no ASAR archives |
| `child_process` | Partial | `spawn`, `spawnSync`, `exec`, `execFile`, `execSync`, `execFileSync`, and `fork`, which runs modules the build finds by their literal paths in the application's own executable, with Node.js's message channel; `fork` does not send handles and does not use `execPath` or `execArgv` |
| `https` | Partial | `request` and `get`, with responses that stream as they arrive and `rejectUnauthorized`; servers and custom agents are not available |
| `net` | Partial | TCP sockets and servers, and Windows named pipes (`\\.\pipe\` names) |
| `tls` | Partial | constants and classes load; secure connections and servers are not available |
| `http2` | Partial | constants and settings helpers load; sessions and servers are not available |
| `crypto` | Partial | MD5, SHA-1 and SHA-2 hashes, HMACs, PBKDF2, HKDF, AES ciphers (ECB, CBC, CTR, GCM), random values, secret keys, X.509 certificates and `webcrypto`; public-key cryptography, Diffie-Hellman and scrypt throw when called |
| `module` | Partial | `createRequire`, `isBuiltin`, `builtinModules` and resolve hooks from `registerHooks`; modules are linked when the application is built, so loader state does not change what loads, and `register()` hooks never run |
| `async_hooks` | Partial | `AsyncLocalStorage` and `AsyncResource` |
| `worker_threads` | Partial | `MessageChannel`, `MessagePort`, `BroadcastChannel` and the main thread's identity, within one process; worker threads cannot be started |
| `zlib` | Partial | deflate and gzip; brotli is not available; the stream classes write their output when the input ends |
| `constants` | Partial | filesystem and Windows constants; OpenSSL constants are not available |
| `tty` | Partial | `isatty` only; terminal streams are not available |
| `dns` | Partial | `lookup` uses the Windows resolver; `resolve`, `reverse` and `lookupService` fail with `ENOTIMP` |
| `perf_hooks` | Partial | marks and measures; observers receive no entries and histograms no samples |
| `vm` | Partial | contexts can be made, but running or compiling code throws, as code cannot be made from strings while the application runs |
| `inspector` | Partial | no debugger attaches: `url()` is undefined, and opening the inspector or connecting a session throws |
| `v8` | Partial | heap statistics are zero and flags have no effect; snapshots and serialization are not available |
| `sqlite` | Not supported | the module loads, but opening a database throws |
| Any other module, such as `cluster`, `dgram` or `repl` | Not supported | |

### Native addons

Native Node.js addons (`.node` files) cannot be loaded. These packages are
implemented natively, so they work without their addon:

| Package | Support | Notes |
| --- | --- | --- |
| `keytar` | Supported | uses Windows Credential Manager, compatible with keytar's storage |
| `native-keymap` | Supported | |
| `ced` | Supported | |
| `clip-filepaths` | Supported | file lists are read from and written to the Windows clipboard as Explorer copies them |
| `node-pty` | Supported | terminals run in Windows pseudoconsoles |
| `native-is-elevated` | Supported | |
| `@vscode/windows-registry`, `@vscode/deviceid`, `@vscode/windows-mutex`, `@vscode/policy-watcher`, `@vscode/windows-process-tree`, `@vscode/windows-ca-certs`, `@vscode/os-proxy-resolver`, `@vscode/native-watchdog` | Supported | |
| `@vscode/sqlite3` | Partial | uses the SQLite that Windows includes; where it is missing or older than 3.38, a built-in engine runs tables, indexes, queries, changes, transactions, pragmas and some `ALTER TABLE` forms, but not all of SQL; `Backup` is not available |
| `@parcel/watcher` | Partial | directory trees are watched; snapshots (`writeSnapshot`, `getEventsSince`) are not written |
| `@vscode/spdlog` | Partial | rotating loggers write files as spdlog does; other logger types write to standard output |
| Other addon packages, such as `better-sqlite3`, `sqlite3`, `serialport`, `sharp`, `canvas`, `bcrypt`, `argon2`, `leveldown`, `nodegit`, `realm`, `usb`, `node-hid`, `robotjs`, `ffi-napi` | Not supported | |

## Web APIs

| API | Support | Notes |
| --- | --- | --- |
| DOM, events, `localStorage`, `sessionStorage` | Supported | |
| `WebSocket` | Partial | `ws:` and `wss:` clients in the main process, utility processes, forked children and pages with Node.js integration; other pages do not have it |
| `ResizeObserver`, `MutationObserver`, `IntersectionObserver` | Supported | |
| `MessageChannel`, `MessagePort` | Supported | ports also move between the main process, preloads, pages and workers |
| `BroadcastChannel` | Partial | where Node.js is available, within one process |
| Web Workers | Partial | dedicated workers run on their page's thread, so long work in a worker holds up the page; a worker script the application makes while it runs, such as a blob, runs only when it just declares, assigns, calls and imports; `SharedWorker` is not available |
| Web Crypto (`crypto.subtle`) | Partial | SHA digests, HMAC, AES-GCM, AES-CBC and AES-CTR, PBKDF2 and HKDF, with secret keys in raw and JWK formats; RSA, elliptic curves and key wrapping are not available |
| WebAssembly | Partial | where `WebSocket` is available; modules run in an interpreter, slower than in Electron; SIMD, threads, exception handling, multiple memories and 64-bit memories are not available |
| Shadow DOM | Partial | shadow trees render with their own style sheets, `:host`, `:host-context()` and `::slotted()`, and retarget events and focus; `adoptedStyleSheets`, `::part()`, `slotchange` and manual slot assignment are not available |
| Custom elements | Partial | autonomous and customized built-in elements upgrade and get their lifecycle callbacks; form-associated elements, `ElementInternals` and `adoptedCallback` are not available |
| HTML drag and drop | Supported | drags inside the app, to and from other apps, and files dropped from Windows |
| Custom drag images (`setDragImage`) | Partial | drags show the platform's image |
| Canvas 2D | Supported | see the limits below |
| `fetch` | Partial | local files and HTTP(S); no CORS or caching |
| `XMLHttpRequest` | Partial | asynchronous requests only |
| `window.open` | Partial | opens a new native window or the default browser |
| Clipboard API | Partial | `readText` and `writeText` work as in Chromium; `read`, `write` and `ClipboardItem` are not available |
| Web notifications | Partial | shown as notification area balloons, which Windows shows as banners, with their icons; `show`, `click`, `close` and `error` events fire; actions are not available |
| `document.execCommand` | Partial | `copy`, `cut`, `paste`, `undo`, `redo`, `selectAll`, `insertText`, `insertParagraph`, `insertLineBreak`, `delete` and `forwardDelete` are supported; other commands are not available |
| WebGL | Not supported | |
| Service Workers | Partial | registered workers install and activate at once; their `fetch` event answers the `fetch()` calls, images and linked style sheets of the pages they control, not their scripts, fonts or navigations; caches live in memory |
| IndexedDB | Not supported | use `localStorage` or the file system |
| Web Audio, media devices (camera, microphone) | Not supported | |

Canvas 2D limits: shadows and filters are not drawn, patterns are not drawn,
compositing modes other than `source-over` and `copy` draw as `source-over`,
conic gradients draw as linear gradients, and `isPointInPath` and
`isPointInStroke` report no hit.

## HTML

| Feature | Support | Notes |
| --- | --- | --- |
| Ordinary elements | Supported | |
| Form controls | Supported | constraint validation checks `required`, `pattern`, email and URL types, number and date ranges and number steps with Chromium's messages; `maxlength` and `minlength` do not report `tooLong` or `tooShort` |
| `canvas` | Supported | |
| Tables, including captions | Supported | |
| `details`, `summary` | Supported | |
| Inline event handlers (`onclick="..."`) | Partial | handlers in the application's pages work; in markup that scripts insert, only handlers whose text is written out in the scripts |
| SVG | Partial | shapes, paths, text, markers, `use` and dashed strokes are drawn, and SVG files used as images (`img` sources and backgrounds) are drawn; gradients, patterns, clip paths, masks, filters, images, text on a path and SVG animations are not |
| `dialog` | Partial | `show()` and `close()` work; `showModal()` opens the dialog in place, without the top layer, a backdrop or making the rest of the page inert |
| `video`, `audio` | Partial | a video shows its poster or its first frame, at its size, and media loads its duration with Chromium's loading events, for formats Windows has codecs for (such as MP4 with H.264, and MP3); media does not play, and controls and captions are not shown |
| `iframe` | Partial | frames show the application's own HTML documents and run their compiled scripts, with `contentWindow`, `contentDocument` and `postMessage`; web pages from other sites are not shown |
| `webview` | Not supported | |
| `object`, `embed` | Not supported | |

## CSS

| Feature | Support | Notes |
| --- | --- | --- |
| Selectors, cascade, custom properties | Supported | including `:is()`, `:where()`, `:not()`, `:has()`, `:nth-*()`, state and form pseudo-classes including `:valid` and `:invalid`, `:lang()`, `:dir()`, `:defined`, `:host`, `:host()`, `:host-context()`, `::slotted()`, `::before`, `::after`, `::marker`, `::placeholder`, `::selection` |
| Flexbox, Grid, `position: sticky` | Supported | |
| Floats | Supported | |
| Tables (`display: table`) | Supported | |
| `display: contents` | Supported | |
| Container queries | Partial | size queries work, also for `::before` and `::after`; style and scroll-state queries do not |
| `@media`, `@supports`, `@import` | Supported | including stylesheets imported from packages |
| Cascade layers (`@layer`), `@property` | Supported | |
| `box-shadow` | Supported | |
| `filter` | Supported | native controls inside a filtered element are drawn unfiltered |
| `backdrop-filter` | Supported | |
| Transforms | Supported | |
| Transitions, `@keyframes` animations | Supported | |
| `clip-path` | Supported | shapes, boxes, `path()` and the document's SVG `<clipPath>` elements; see the limit below |
| Masks (`mask`, `mask-image` and the other `mask-*` properties, also with `-webkit-`) | Supported | images, SVG images and gradients; see the limit below |
| `font-variant-numeric`, `font-feature-settings` | Partial | applied to system fonts; text in web fonts is drawn without the features |
| Scrollbar styling (`::-webkit-scrollbar` parts, `scrollbar-color`, `scrollbar-width`) | Supported | background images in scrollbar parts are not drawn |
| Linear gradients | Supported | one opaque layer with an angle or side and increasing percentage stops |
| Other gradients | Partial | corner directions, transparent stops, length or hard stops, repeating and radial gradients and multiple layers are approximate; conic gradients are not drawn |
| Right-to-left text (`direction`, `unicode-bidi`, `dir`) | Partial | text, lines, carets and selections follow the direction; grid columns, block margins, list markers and scrollbars keep their left-to-right sides |
| `clip-path: url(...)` to another document | Partial | does not clip |
| `mask` referencing an SVG `<mask>` element | Partial | does not mask |
| `@font-face` | Partial | fonts in the application's files and data URLs load, including WOFF, WOFF2 and variable fonts, with `unicode-range` and weight ranges; fonts at `http` and `https` addresses and fonts scripts add with `FontFace` are not loaded |
| `text-shadow` | Partial | not drawn |
| `overflow-anchor` | Partial | scroll positions are not adjusted when content above changes |
| Unknown properties | Partial | ignored |
| Other pseudo-classes, such as `:visited`, `:target`, `:popover-open`, `:state()` | Not supported | they never match |
| `mix-blend-mode` other than `normal` | Not supported | |
| `filter: url(...)` | Not supported | |
| Multi-column layout | Not supported | |

## Known differences

- Text uses ClearType, and grayscale where Chromium uses it: in scrolling
  areas and elements with backdrop filters that Chromium draws as layers of
  their own, under filters and in text fields. Electron may draw text in some
  other layers, such as animated ones, in grayscale.
