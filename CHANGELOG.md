# Changelog

## 0.1.0-alpha.7

- Bundled applications (electron-vite, vite-plugin-electron, Forge with
  Vite) ship only the files their main process, preloads and forked
  modules name: MarkText's output is back to 27 files from 1,191 with
  0.1.0-alpha.6. In every layout, test folders and their fixtures,
  packages' style sheet sources and native build leftovers stay behind,
  and the application's `files` exclusions apply to package files, as
  electron-builder applies them.
- The application's files are in `resources\app` beside the executable,
  as in a packaged Electron application: `app.getAppPath()`, `__dirname`
  and module paths point there. `extraResources` are in `resources` and
  `extraFiles` beside the executable.
- Scripts that pages build as text from a template literal and insert as
  `<script>` elements run, compiled ahead of time, as VS Code's webviews
  define `acquireVsCodeApi`.
- An inserted classic script runs in a task of its own and fires `load`
  right after, so AMD loaders get each script's `define`.
- `fork()` children get the module path in `process.argv`, as in Node.
- TypeScript that its build compiles to CommonJS calls the module's
  `require` also when it declares `require` for TypeScript.
- CSS: `grid-template-areas` and named grid lines in `grid-area`,
  `grid-row` and `grid-column`; `border-inline-*` and `border-block-*`;
  Chromium's `-webkit-margin-*` and `-webkit-padding-*` names. Properties
  with `-moz-`, `-ms-` and `-o-` prefixes are ignored, as Chromium
  ignores them.
- Text in web fonts that asks for OpenType features, such as
  `font-variant-numeric: tabular-nums`, is drawn in its font without the
  features instead of as missing-glyph boxes.
- Repainting a small area, such as a caret's blink, draws only what is in
  that area.

## 0.1.0-alpha.6

- Fixes builds that failed with `error[E0502]: cannot borrow *realm as
  mutable` in a generated crate, as MarkText's did with 0.1.0-alpha.5:
  functions whose source text the program reads compile again.
- Pages show `<iframe>` documents: each frame runs its document's compiled
  scripts in a realm of its own, painted and sized at the frame's box,
  with `contentWindow` and `contentDocument` for same-origin frames,
  `postMessage` between frames, `load` events, `requestAnimationFrame` and
  `document.open`, `write` and `close`. Frames are 300 by 150 pixels with
  an inset border unless styled. The `documents` setting names HTML files
  that pages show in frames by URLs they build while they run.
- Service workers: pages register them with `navigator.serviceWorker`,
  and a worker installs, activates, claims its pages and exchanges
  messages with them. Its `fetch` event answers the `fetch()` calls,
  images and linked style sheets of the pages it controls, and it has
  `clients` and in-memory `caches`. Markup a page writes with
  `document.write` loads its images, style sheets and handlers.
- Functions run in the realm that made them, so a frame calling its
  page's listener sees the page's globals.
- CSS container queries: `container-type`, `container-name`, the
  `container` shorthand and `@container` rules with size conditions, also
  for `::before` and `::after`.
- `PerformanceObserver` reports the page's marks and measures.
- `net.isOnline()` and `net.online` report whether Windows has a network
  connection, and `windows-foreground-love` and `@vscode/fs-copyfile` are
  supported native modules.
- Paths through `app.asar` or `*.asar.unpacked` read the folders those
  archives were packed from, as packaged Electron applications expect.
- The `forks` setting names modules a program starts with `fork()` by
  paths it builds while it runs, and forked modules get files at those
  paths, so programs that check a module exists before they fork it work.
- The `scripts` setting names scripts that pages load by URLs they build
  while they run, such as those of HTML the application generates, with
  the source each compiles from.
- `createRequire(import.meta.url)` kept as a value, as esbuild's ESM
  bundles keep it, requires modules by name when the application runs.
- Packages around plugin packages that keep what they share, such as an
  editor's `extensions/node_modules`, get their dependencies installed.

## 0.1.0-alpha.5

- NativeBoat is open source under the MIT license. Its source, releases and
  documentation are in the [nativeboat](https://github.com/XTSoftwareLabs/nativeboat)
  repository, and built applications' `THIRD-PARTY-NOTICES.txt` includes
  NativeBoat's own MIT notice.
- HTTPS responses stream as they arrive: `data` events follow the
  download, a response read slowly pauses it, and large downloads such as
  updates no longer wait whole in memory. `rejectUnauthorized: false`, on
  the request or its agent, and `NODE_TLS_REJECT_UNAUTHORIZED=0` accept
  server certificates Windows does not trust, as in Node.
- Form controls check their constraints as Chromium does: `required`
  (radio groups and select placeholders included), `pattern`, email
  addresses and URLs, number and date ranges and number steps, with
  Chromium's messages in `validationMessage`. `:valid` and `:invalid`
  match controls, forms and fieldsets, and `setCustomValidity` makes a
  control invalid.
- `-webkit-text-fill-color` fills text, `font-kerning: none` keeps glyph
  pairs unkerned, and `-webkit-user-drag` decides whether pressing and
  moving over an element drags it: `none` keeps images and links still and
  `element` drags any element.
- Drawn selects place their arrow where Chromium does, 8.5 pixels inside
  the right border whatever the padding, and checkboxes and radio buttons
  made fully transparent, as switches lay them over their track, show the
  page beneath them and still take the clicks.
- `::before` and `::after` boxes in the flow paint their backgrounds and
  borders, so an empty dot drawn with `content: ""` shows.
- Text fields with transparent text show what the page draws beneath
  them, with their caret and placeholder, as fields laid over a
  highlighted copy of their text expect; translucent field text blends as
  CSS colors do.
- `text-underline-offset` places underlines that far below the baseline.
- In preloads, `process.nextTick` callbacks the page's code queues run
  after the page has rendered, as on Electron's next Node turn, so work a
  library starts that way no longer holds back the page's next frame.
- Main processes and preloads have Node's global `performance`: `now()`,
  `timeOrigin`, marks and measures with their entries, and `toJSON()`.
- `nativeboat build --config <FILE>` reads a settings file for the native
  build, so the project needs no change: its `publish` names the feed native
  builds update from, before package.json's `nativeboat.publish`.
- Checkboxes and radio buttons look as in Chromium, and fill with the
  page's `accent-color` when checked.
- The pointer over a form control moves over the page: `:hover` styles,
  `mousemove`, `mouseover` and `mouseenter` events and the hover look of
  checkboxes follow it, and controls show the cursor their style asks
  for, such as `cursor: pointer`.
- `child_process.spawnSync` works as in Node: it returns the process's
  `pid`, `status`, `stdout`, `stderr` and `output`, as `Buffer`s or in the
  `encoding` asked for, and a program that cannot start gives an `error`
  with Node's `code`, such as `ENOENT`.
- A label the pointer is over or presses hovers or presses its control
  too, as in Chromium: `:hover` and `:active` styles and the hover look of
  a checkbox follow the pointer over its label.
- `nativeboat analyze` reports React as Partial and `display: contents` as
  Supported, as the compatibility list does, and its notes for web fonts
  and web notifications say what works.
- Applications that keep only some Electron locales with
  electron-builder's `electronLanguages` take their locale from those, as
  Electron does from the resources it ships: an application keeping only
  `en-US` reports `en-US` from `app.getLocale()` and `navigator.language`
  on a Chinese Windows, and uses English default fonts. `monospace` is
  Courier New before Electron 36, which made it Consolas where text uses
  ClearType.
- Single-line text fields stay in the middle of their line when the page
  around them scrolls or changes size; their text and placeholder used to
  jump to the top of the line.
- Buttons laid out as blocks or flex containers in the flow of text are
  as wide as their content when their width is `auto`, as in Chromium,
  rather than as wide as their container.
- Styled selects are as tall as in Chromium: their label's line is the
  height of the primary font's, whatever fonts the label falls back to,
  with a device pixel above and below it unless `appearance: none`.
- `app.getPreferredSystemLanguages()` lists the languages of Windows'
  language settings, such as `zh-Hans-CN`, and pages' `navigator.languages`
  is the application's locale followed by them, as in Electron.
- `nativeboat build` prints the project's warnings, such as electron-builder
  settings it does not use. Applications that open no windows, such as tray
  applications, are not warned that no page was found.
- Message boxes look and answer as Electron's: a message without a
  `detail` is the dialog's content, buttons named OK, Cancel, Yes, No,
  Retry or Close are the dialog's own and other buttons are command links
  unless `noLink` is set, an empty title shows the application's name,
  `defaultId` and `cancelId` take Electron's defaults, and
  `checkboxLabel` adds a checkbox whose state the result reports.
  `dialog.showOpenDialogSync`, `showSaveDialogSync` and
  `showMessageBoxSync` are available, and a canceled save dialog gives an
  empty `filePath`.
- `safeStorage` encrypts and decrypts as Electron does on Windows: its key
  is protected for the user by Windows and kept in `Local State` in the
  user data directory, so text an Electron build of the application
  encrypted decrypts in the native build. `powerSaveBlocker` keeps the
  computer, or also the display, from sleeping.
- `powerMonitor` reports idle time and state, whether the computer runs on
  its battery, and emits `suspend`, `resume`, `on-ac`, `on-battery`,
  `lock-screen` and `unlock-screen`. `systemPreferences` gives the accent
  color, Windows' system colors, animation settings and media access, and
  emits `accent-color-changed` and `color-changed`, as in Electron.
- The `webContents` module finds the contents of the application's
  windows: `getAllWebContents`, `fromId` and `getFocusedWebContents`.
- Windows take the `icon` option and `setIcon`, and windows without one
  show the application's icon. `autoHideMenuBar` hides the menu bar until
  Alt shows it, with `setMenuBarVisibility`, `setAutoHideMenuBar`,
  `isMenuBarVisible` and `isMenuBarAutoHide`. The `transparent`,
  `alwaysOnTop` and `skipTaskbar` options work, and Electron's macOS-only
  window methods, such as `setSimpleFullScreen` and `isDocumentEdited`,
  exist and behave as Electron's do on Windows.
- Menus look like Electron's: rounded corners, Chromium's shadow, and
  shortcuts and submenu arrows sharing the right column.
- Rounded corners, elliptical ones included, are antialiased as in
  Chromium, for backgrounds, borders and clipped content. Thin strokes,
  such as those of icons, keep their weight.
- SVG files work as images again, in `<img>` and as backgrounds, drawn as
  vectors at any size.
- Videos show their poster or first frame and report their duration and
  loading events, with Chromium's colors and scaling.
- `nativeImage` behaves as Electron's on Windows: JPEG and icon files,
  thumbnails at the size asked for, `resize` and `crop` as Chromium does
  them, `toJPEG`, scale factors and representations, and Electron's
  errors.
- Shadow DOM: `attachShadow`, slots, and styles, layout and painting of
  shadow trees, with `:host` and `::slotted()`. Custom elements:
  `customElements.define`, upgrades and the lifecycle callbacks.
  `:lang()`, `:dir()` and `:nth-child(An+B of S)` match, and `@supports`
  accepts declarations whose values use `var()`.
- Pages can start dedicated workers. They run on the page's thread, each
  with a worker global of its own, and exchange messages and errors with
  the page. A worker can be made from a function's text, since
  `Function.prototype.toString` returns the source of functions a program
  reads as text.
- Message ports: `MessageChannel`, `MessagePort` and `BroadcastChannel`
  in Node code and in `worker_threads`. Electron's `MessageChannelMain`
  ports pass between the main process, preloads and pages through
  `webContents.postMessage`, and `utilityProcess` and
  `child_process.fork` run modules as child processes with message
  channels.
- Node's `crypto` module runs on Windows' cryptography (CNG): MD5, SHA-1
  and SHA-2 hashes, HMACs, PBKDF2 and HKDF, AES in ECB, CBC, CTR and GCM
  modes, random values, `timingSafeEqual`, secret keys and X.509
  certificates. Every page and process has the Web Crypto API:
  `getRandomValues`, and `crypto.subtle` with SHA digests, HMAC, AES-GCM,
  AES-CBC, AES-CTR, PBKDF2 and HKDF; other algorithms reject with
  `NotSupportedError`.
- Main processes and preloads have Node's `WebSocket` client for `ws:`
  and `wss:` URLs, through the system's TLS and proxy settings.
- `@vscode/sqlite3` databases run on Windows' own SQLite (3.38 or later)
  when the system has it, and otherwise on a built-in engine that covers
  tables, indexes, constraints, queries, upserts, transactions, pragmas and
  `ALTER TABLE ... ADD COLUMN` and `RENAME TO`.
- WebAssembly modules load and run, with the JavaScript API over them.
  They are interpreted, so they run slower than in Electron.
- More of Node: `net` servers and clients over Windows named pipes,
  `vm` contexts (without evaluating code strings), `inspector` (with no
  debugger attached), `module` resolve hooks and `createRequire`,
  `diagnostics_channel`, `perf_hooks`, `v8`, `dns`, `util/types`,
  `process.memoryUsage`, `cpuUsage`, `uptime`, `availableMemory`,
  `release` and `config`, and a `navigator` global outside pages. A child
  process's output streams give `Buffer`s until `setEncoding` is called.
- More of Electron: the `protocol` module's `handle`,
  `registerFileProtocol`, `registerBufferProtocol` and
  `registerStringProtocol` serve style sheets, images, fonts, media and
  `fetch` to pages. `app` has `enableSandbox`, the logs path, `focus`,
  metrics, GPU status, jump list settings, accessibility support, proxies
  and protocol clients, and every window shares one default session.
  `WebContentsView` builds with a warning and throws if used.
- `nativeboat analyze` reports Windows named pipes in `net`, the system
  SQLite behind `@vscode/sqlite3`, and top-level `await` as Partial.
- Native addons the runtime provides in place of their binaries:
  `@vscode/windows-registry`, `@vscode/deviceid`, `@vscode/windows-mutex`,
  `@vscode/policy-watcher`, `@vscode/windows-process-tree`,
  `@vscode/windows-ca-certs`, `@vscode/os-proxy-resolver`,
  `@vscode/spdlog`, `@vscode/sqlite3`, `@vscode/native-watchdog`,
  `@parcel/watcher`, `node-pty` and `native-is-elevated`.
- JavaScript and TypeScript: top-level `await` (a dynamic `import()` waits
  for the module, a static import does not), `using` and `await using`
  declarations, TypeScript classes with `experimentalDecorators`, enums
  and namespaces as `tsc` compiles them, `import x = require()`, and
  `import()` of a specifier computed when the program runs, among the
  modules the build can find. A `require` the build cannot satisfy warns
  at build time and throws `MODULE_NOT_FOUND` when it runs, as in Node.
- Projects: plugin packages, named by folder globs in the `plugins`
  setting of a settings file or package.json's `nativeboat` key, are
  found, get their dependencies installed and compile with the
  application. Projects that download Electron themselves are recognized,
  and gulp or grunt tasks that produce files the project's stylesheets
  name run before compiling. An application's modules and installed
  packages run from the executable's folder, as a packaged application's
  do, with the files its production dependencies read at run time beside
  them.
- Large applications build: very large modules and long chains of
  operators compile, and long toolchain command lines no longer fail. The
  build cache takes about half the disk it did, and rebuilding one part
  of an application no longer recompiles the parts that depend on it.
  `NATIVEBOAT_KEEP_GOING` reports every part that fails to compile instead
  of stopping at the first.
- `fs.watch` and `@parcel/watcher` report changes made right after a watch
  starts; on a busy computer the first changes could go unreported. A
  forked child that sends a message and then disconnects, as in
  `process.send(message, () => process.disconnect())`, delivers the
  message before its parent sees `disconnect`.
- `nativeboat analyze` judges a window's `webContents` and the default
  session member by member, reports `navigator.clipboard.readText` and
  `writeText` as supported and the rest of the Clipboard API as partial,
  leaves Vue's `:deep()`, `:slotted()` and `:global()` to Vue, and parses
  TypeScript as TypeScript.

## 0.1.0-alpha.4

- Applications update themselves with electron-updater. The build writes
  `resources\app-update.yml` from electron-builder's `publish` settings,
  taking the GitHub repository from package.json or the `origin` remote as
  electron-builder does, and makes a Windows installer with the update
  metadata (`latest.yml`) beside it. Native builds take their own feed from
  `nativeboat.publish` in package.json. The installer installs for the
  current user with shortcuts, an uninstaller and an Installed apps entry,
  takes NSIS's `/S` and `/D=`, and installs updates into the running
  application's folder once it exits.
- `net.request` works as in Electron: requests are writable streams,
  redirects are followed, refused or handed to the `redirect` event as the
  `redirect` option says, and responses are readable streams with the
  status, headers and body. `session.fromPartition` gives the session
  every partition shares.
- Child processes have `ref()` and `unref()`: an unreferenced child no
  longer keeps a Node program running.
- Electron's `autoUpdater` behaves as in applications Squirrel.Windows did
  not install: it keeps its feed URL, checks report that Squirrel is
  missing, and its events reach their listeners.

- The application's files that electron-builder's `files` setting selects,
  such as its pages, style sheets, images, translations and the programs
  it starts, are copied beside the executable, without the sources the
  build compiles into it. Pages load the style sheets and images they link
  to, and the application finds its files wherever it is installed. The
  application's own CommonJS modules see `__dirname` and `__filename` in
  that folder, and `loadFile` takes a relative path from it, as in
  Electron.
- `require('original-fs')` gives `fs`, as compiled applications have no
  ASAR archives.
- `app.getSystemLocale()` reports the Windows regional format, such as
  `en-US`.
- `session.resolveProxy(url)` reports the proxy the Windows settings give
  for a URL, from a configuration script, automatic detection or a fixed
  server with its bypass list, as `DIRECT`, `PROXY host:port` or
  `SOCKS host:port`.

- Libraries a page loads with classic `<script>` tags in their universal
  module form, such as SlimSelect, set their globals as in a browser:
  `module` and `exports` are only the page's own.
- `offsetTop` and `offsetLeft` are measured from the offset parent's
  padding edge in layout positions, as in Chromium: scrolling a list does
  not change its items' offsets, so widgets that scroll an item into view
  land on it.
- Pages that fade text or shadows with `opacity` repaint up to ten times
  faster: only the area the faded content covers is composited, not the
  whole window.

- Applications show their first window sooner: `process.ppid` no longer
  lists every process on the system at startup.
- JSON Schema validation (Ajv, as conf and electron-store use it) works out
  once per schema which keywords apply, so settings stores validate faster.
- Content faded out to `opacity: 0`, such as a closed drop-down list, is
  not painted.
- Text that sets no font uses the standard font of the user's language, as
  Chromium does: Microsoft YaHei on Simplified Chinese Windows, Times New
  Roman on English Windows.
- Each script of a page ends in a microtask checkpoint, as in browsers, and
  replies to IPC the script requested arrive before the next script runs,
  as they do in Electron while the page's other scripts load. Pages whose
  later scripts read what an earlier script awaited, such as a saved
  language, now see it on the first launch.

- Default and fallback fonts follow the Electron version the application
  uses: before Electron 32, Chinese, Japanese and Korean text uses the
  system fonts Chromium then chose (Microsoft YaHei on Chinese Windows)
  rather than Noto CJK fonts, so lines are as tall as in the application's
  Electron.
- Text symbols such as a heart without an emoji selector are drawn in a
  symbol font, as Chromium draws them, and before Electron 34 an emoji
  selector keeps a character in a page font that has it.
- Bold text at weight 600 in system fonts uses their bold face, such as
  Microsoft YaHei Bold, as Chromium does.
- Images from `http` and `https` addresses load: they download in the
  background and fire `load` or `error` when they arrive, and the page
  repaints with them.
- WebP images are shown on every Windows edition, and images in other
  formats Windows has a codec for are shown too.
- Images and inline event handlers (`onload="..."`, `onclick="..."`) in
  markup that scripts insert with `innerHTML`, `insertAdjacentHTML`,
  `outerHTML` or `createContextualFragment` work as in Chromium, when the
  handler's text is written out in the scripts. The page's own images fire
  `load` and `error`.
- `Intl.DisplayNames` names languages, regions, scripts, currencies,
  calendars and date fields in the requested language.
- Pages can show notifications with the web `Notification` API, which
  Electron allows by default; they appear as the application's
  notifications, and clicking one fires its `click` event.
- An error nothing catches in a preload script's callback is reported on
  the console and the application keeps running, as in Electron; it used
  to close the application.
- Background images and other images style sheets name at `http` and
  `https` addresses download and show.
- Web notifications show their icons: files, data URLs and images from the
  network.
- Images follow `object-fit` and `object-position`, and clip to rounded
  corners.
- Rounded borders whose sides differ in width, color or style keep their
  corners, and small round boxes no longer show lines across them.
- Variable web fonts show the weights pages ask for, and `@font-face`
  `unicode-range` and weight ranges choose between a family's faces.
- `color-mix()` in every color space, `lab()` and `lch()`, gradients'
  interpolation spaces, and `calc()` with `infinity`, `pi` and `e`.
- Scrollbars look and behave as the application's Chromium release draws
  them: Fluent scrollbars since Chromium 135, buttons and tracks that repeat
  while held, no thumb when nothing scrolls, and window scrollbars that take
  room from the page. Media queries, viewport units and `innerWidth` see
  the whole window, as in Chromium.
- Text and boxes land on the same device pixels as in Chromium, text is
  smoothed in grayscale where Chromium smooths it so (composited layers,
  filters, text fields), and windows take the size Electron gives them at
  fractional display scales.
- Content under a blurred backdrop, such as a toast under a dialog,
  repaints without dark edges when it changes.
- Requests send the user agent Electron and Node send.
- `--user-data-dir` sets the application's user data folder, as in
  Electron.
- Computed styles report logical properties, such as `margin-inline-start`,
  as the physical sides they map to.
- `Intl.Locale` works, and errors from calling something that is not a
  constructor name it, as V8 does.
- `webFrame.setVisualZoomLevelLimits` takes two numbers, as in Electron.
- fflate's asynchronous API works.
- Spell checking no longer fails when it is used from another thread.

## 0.1.0-alpha.3

- `assert` has Node's whole API: `equal`, `notEqual`, the deep and strict
  comparisons, `throws`, `rejects`, `doesNotThrow`, `match`, `ifError`,
  `fail`, `AssertionError` and `assert.strict`. Deep equality is Node's own.
- `process.binding('uv')` gives libuv's error codes, `errname` and
  `getErrorMessage`, as execa expects.
- `ipcMain` is a full event emitter: `once`, `off`, `removeListener`,
  `removeAllListeners`, `prependListener`, `listenerCount` and the rest,
  plus `handleOnce` and `removeHandler`.
- `clipboard.readImage()` returns the clipboard's picture, from its PNG form
  or its bitmap. `nativeImage` gains `toPNG`, `toDataURL`, `createEmpty`,
  `createFromDataURL` and `createFromBitmap`, and windows gain
  `setPosition` and `getPosition`.
- The `clip-filepaths` package works without its native addon: file lists
  are read from and written to the Windows clipboard as Explorer copies
  them, with the package's results and error messages.
- The Fetch API works in the main process and preloads as in Node: the
  `fetch`, `Headers`, `Request`, `Response` and `FormData` globals, with
  response bodies as streams, gzip and deflate decoding, redirects,
  `data:` URLs and aborting. Classes that extend `Request` or `Response`,
  as Hono's Node adapter does, work too.
- Web streams work: `stream/web` and the `ReadableStream`,
  `WritableStream`, `TransformStream`, `TextDecoderStream`,
  `CompressionStream` and other globals, `Readable.toWeb` and
  `Readable.fromWeb`. `Blob`, `File` and `DOMException` are globals too,
  also when read as `globalThis.X`.
- Errors thrown by Node's library code carry Node's codes, classes and
  messages for every code, where some codes were missing before.
- `net` sockets and servers work: TCP connections and listening servers
  with Node's stream events, timeouts, keep-alive, `ref`/`unref` and
  errors (`ECONNREFUSED`, `EADDRINUSE`, ...). Named pipes are not
  available yet.
- `http` servers and clients work as in Node: `createServer` with
  keep-alive, pipelining, chunked bodies, `Expect: 100-continue`,
  `upgrade` and `connect` events, and `request`/`get` with agents, socket
  reuse, timeouts and `CONNECT`. `https` requests go through the system's
  HTTPS client and proxy; their response bodies arrive whole. HTTPS
  servers are not available. `nativeboat analyze` reports `http` as
  supported.
- `zlib` has Node's `constants`, `codes` and legacy constant names, the
  `Deflate`, `Gzip`, `Unzip` and other classes as transform streams (they
  write their output when the input ends), and `crc32`. Decompression reads concatenated gzip members, finishes input
  that ends early when `finishFlush` asks for it, as HTTP clients do, and
  fails with zlib's messages, codes and numbers.
- `app.setLoginItemSettings` and `app.getLoginItemSettings` work as in
  Electron on Windows: the item is a `Run` entry named after the
  AppUserModelId that `app.setAppUserModelId` now sets, disabled items are
  marked as the Task Manager marks them, and `launchItems` lists the
  application's entries.
- `process.binding()` gives the bindings Node still serves to old
  packages, such as `constants`, `fs`, `os` and `zlib`.
- Tailwind CSS 4 stylesheets build as `@tailwindcss/vite` builds them: with
  the project's own `tailwindcss` compiler and the plugins and
  configurations their `@plugin` and `@config` rules name, from the class
  names in the renderer's sources and `@source` folders.
- Main-process and preload sources that electron-vite, vite-plugin-electron
  or Forge's Vite plugin bundle into CommonJS can call `require()`, also
  inside functions, and their `__dirname` and `__filename` are the bundle's
  in the application, as when the bundle runs.
- electron-builder configurations written as JavaScript or TypeScript
  modules (`electron-builder.config.ts` and the like) supply the
  application's identity, resources and output folder.
- Cascade layers order the cascade as in browsers: later layers win over
  earlier ones in the order `@layer` statements and blocks declare them, a
  layer's own rules over its sublayers', rules outside any layer over
  layered ones, and the other way round for `!important` declarations.
  `@property` registers custom properties: elements start from their
  initial values, and from their parents' only when they inherit.
  `nativeboat analyze` reports both as supported.
- Stylesheets' `@import` rules work, in stylesheets imported from scripts
  and in Vue components' styles: the imported sheets, relative or from
  packages by their `style` export condition or field, are inlined with
  their `layer()`, `supports()` and media conditions, and their files,
  such as fonts, travel with them. Imports after other rules are ignored,
  as in browsers.
- `require()` calls whose module is computed when the application runs,
  such as plugin loaders, no longer stop the build: `nativeboat build`
  warns about each one, and when it runs it loads a module the build
  compiled by that name, or throws Node's `MODULE_NOT_FOUND` error.
- `Tray` shows icons in the notification area with their tooltips, images
  and balloons; clicks, right clicks, double clicks and balloon clicks reach
  the `Tray`'s listeners, and the menu given to `setContextMenu` or
  `popUpContextMenu` opens beside the icon, drawn as windows' menus are.
  `Notification` shows notifications as notification area balloons, which
  Windows shows as banners, with `show`, `click` and `close` events.
  `globalShortcut` registers system-wide shortcuts, and
  `protocol.registerSchemesAsPrivileged` accepts the application's schemes.
  `nativeImage.createFromPath` reads PNG files.
- `AsyncLocalStorage` from `node:async_hooks` keeps its store across
  `await`s, promise callbacks, `process.nextTick`, timers, immediates and
  I/O callbacks, as in Node, and `AsyncResource` runs callbacks in the
  context it was created in.
- `require()` calls in CommonJS `try` blocks that name a module the build
  cannot find, such as optional dependencies and other platforms' native
  packages, throw Node's `MODULE_NOT_FOUND` error when they run instead of
  stopping the build. Requests in branches that build-time defines rule out,
  such as React's development build under `process.env.NODE_ENV ===
  'production'`, are not compiled. `Object.defineProperty(module, 'exports',
  ...)` sets a module's exports.
- `process.ppid`, `process.umask()`, `process.report.getReport()` and
  `process.stdin` work in the main process; `columns` and `rows` of
  `process.stdout` and `process.stderr` read as `undefined`, as a windowed
  application's are.
- Node's `readline` and `readline/promises` modules work. `http2`, `net` and
  `tls` load, with Node's constants, settings and address checks; their
  sessions, sockets and servers report that they are not available.
  `nativeboat analyze` reports the Electron APIs and Node modules compiled
  applications provide: `Tray`, `Notification`, `globalShortcut`,
  `protocol`, `powerMonitor`, `safeStorage` and the `webContents` module, and
  Node's `dns` and `perf_hooks`, as not supported.
- JSX in `.jsx` and `.tsx` modules compiles, to React's automatic runtime
  or to the factory the TypeScript configuration's `jsx`, `jsxImportSource`,
  `jsxFactory` and `jsxFragmentFactory` name.
- Imports in TypeScript modules that no value uses are dropped, as
  TypeScript and Vite drop them, unless `verbatimModuleSyntax` is on: types
  imported without `import type`, and modules that only declare types, no
  longer stop the build. `declare module` blocks are ignored.

## 0.1.0-alpha.2

- Lines of text within a box sit on the device pixel rows Chromium puts
  them on when they fall half way between rows: their positions are kept in
  64ths of a CSS pixel before scaling, as Chromium's are.
- `clip-path` clips to polygons, ellipses, `path()` shapes and the
  document's SVG `<clipPath>` elements exactly, for painting and for the
  pointer; they clipped to their bounds or a rounded rectangle before.
- CSS masks: `mask-image`, `mask-size`, `mask-position`, `mask-repeat` and
  the `mask` shorthand, also with the `-webkit-` prefix, keep what the
  mask image's alpha covers, for images, SVG images and gradients.
  `nativeboat analyze` reports masks as supported, except references to
  SVG `<mask>` elements.
- Compiled applications are smaller: identical functions are folded, and
  module setup code no longer grows with the number of modules times their
  dependencies (MarkText: 299 MB to 287 MB).
- Boxes with a negative `z-index` inside positioned boxes that make no
  stacking context paint, and take the pointer, beneath the content of the
  enclosing stacking context, as in browsers. MarkText's hidden source of a
  diagram no longer covers the block after it, so a `<summary>` there
  opens on a click.
- Mouse moves that Windows repeats at the same place while windows change
  under a still pointer no longer reach pages as `mousemove` events, as in
  Chromium. Hover menus, such as MarkText's paragraph button, no longer jump
  to whatever scrolls under a resting pointer.
- Content that scrolls under a still pointer updates the hover with
  `mouseover`, `mouseout`, `mouseenter` and `mouseleave` events (and their
  pointer events), but no `mousemove`, as in Chromium. MarkText's paragraph
  menu button no longer jumps to the block that scrolls under the pointer.
- `getComputedStyle` reports line heights in pixels, and the width, height,
  margins and paddings of laid-out boxes as layout used them (`auto` widths,
  percentages and auto margins in pixels), as in Chromium. It also reports
  the `margin`, `padding`, `inset`, `border-*`, `overflow`, `gap` and
  `flex` shorthands.
- `IntersectionObserver` reports when observed elements come into view,
  leave it or move, as pages scroll and their layout changes, with root
  margins and thresholds. Buttons and toolbars that follow an element, such
  as MarkText's paragraph menu button, move with it when the content above
  shifts. `nativeboat analyze` reports `IntersectionObserver` as supported.
- A word that may not start a line, such as a comma after inline code or
  a link, no longer overflows the line or breaks off alone: the line
  breaks at its last earlier opportunity and the text from there moves
  down with the word, as in Chromium.
- `::marker` rules style list item and summary markers: their color and
  font, as MarkText's dark themes color list numbers and bullets.
- SVG strokes can be dashed with `stroke-dasharray` and
  `stroke-dashoffset`, and `marker-start`, `marker-mid`, `marker-end` and
  the `marker` shorthand work as CSS properties, set in `style`, style
  sheets or attributes, and inherited. Sequence diagrams show their dashed
  replies and arrowheads, and Mermaid its dotted edges.
- Moving the caret with the arrow keys, Home and End, or typing, in
  editable content scrolls the caret into view, as in Chromium: each
  scroller around it scrolls the least that shows it. Selections that
  scripts set leave scrolling alone.
- Text in scroll containers that Chromium draws as layers of their own, those
  holding boxes with a negative `z-index` beneath their content, is smoothed
  in grayscale as in Chromium, where nothing opaque lies beneath it; it
  showed ClearType's colored fringes.
- Lines of text stack as in Chromium when their line height is fractional,
  as `line-height: 1.6` often makes it: at some display scales some lines'
  text sat a device pixel lower than the others.
- Table captions sit above their tables, or below them with
  `caption-side: bottom`, outside the table's border, and the table's
  margins go around them, as in browsers. `nativeboat analyze` reports
  captions as supported.
- Form controls inside rotated or skewed boxes can be used while the
  transform lasts: they show turned with the box, and clicks, typing and
  selection reach them; a date picker's calendar drops down below the
  turned field. `nativeboat analyze` reports CSS transforms as supported.
- Scroll containers inside scaled or rotated boxes can be used while the
  transform lasts: the wheel scrolls them, and their scrollbars, drawn as
  Chromium draws them or as author CSS styles them, follow the transform
  and can be pressed and dragged.
- Backdrop filters filter the frames of native text fields and the native
  scroll bars of scroll containers behind them too; they stayed sharp.
  `nativeboat analyze` reports backdrop filters as supported.
- Tables size their columns as browsers do: cells fill their columns,
  widths cells give in lengths or percentages size the columns, room to
  spare goes to automatic columns first, and `table-layout: fixed` takes
  the first row's widths. `nativeboat analyze` reports tables as supported.
- Pages without scripts show their content in compiled applications; they
  were blank.
- Fixed flickering in application windows: scrolling areas and controls
  flashed white whenever the content around them changed, for example while
  the mouse moved over an editor.
- `nativeboat analyze` reports Node's `crypto` module by the functions used
  (hashes, HMACs, random bytes and UUIDs are available), and no longer
  reports `appearance: none` as unsupported. It reports `<details>` and
  `<summary>` as supported, and `<dialog>` by what `showModal()` lacks.
- CSS transitions and keyframe animations run in compiled applications,
  including on `::before` and `::after`, and transitions of inherited
  properties such as `color` reach the elements that inherit them.
  `z-index` animates too, so animations can change which element is on
  top, as do background sizes and positions, transform origins, clip paths,
  grid templates and the other properties Chromium interpolates; background
  images cross-fade.
- `font-variant-numeric` and `font-feature-settings` select OpenType
  features such as tabular numbers.
- Text areas, and scroll containers whose `resize` allows it, show a resize
  grip in their corner; dragging it resizes them.
- Text wraps around floats: lines beside a float are shortened, and lines
  below it take the full width again.
- Scroll containers and form controls inside scaled or rotated boxes scale
  and rotate with them, as dropdowns and dialogs do while they zoom in;
  scaled form controls can still be typed in and clicked.
- Files dragged from Explorer onto a compiled application's window reach
  its pages, as when dropping a document or an image onto MarkText; GIF,
  WebP, SVG, BMP and AVIF images arrive with their image types.
- Text, links and HTML dragged from other applications reach pages with
  their drags and drops, and editable content takes dropped text where it
  is dropped, as in Chromium.
- Copying, cutting and pasting reach the system clipboard in compiled
  applications' pages, and scripts that select text in text fields select
  it there. Edit menu roles and `webContents` edit commands (copy, paste,
  undo, redo, select all...) run in the focused page, so MarkText's Copy and
  Paste, and their shortcuts, work.
- Selects that pages style and draw open their options below them in
  compiled applications, and the option chosen becomes the select's value.
- Canvases draw in compiled applications' pages: 2D drawing, images,
  reading and writing pixels, `toDataURL()` and text measurement work as in
  the development renderer.
- `localStorage` persists across runs of compiled applications, in the
  application's user data, as MarkText's sidebar width and export settings
  do in Electron.
- Drags that leave the window continue in other applications: dragged
  text, links and HTML drop into editors, browsers and Explorer, and text
  moved out of editable content leaves it. `nativeboat analyze` reports drag
  and drop as supported, and custom drag images separately.
- Selected text can be dragged: dropped in editable content it moves there,
  or with Ctrl held is copied, and a press on the selection that does not
  drag it puts the caret there.
- Right-to-left text: `direction`, `unicode-bidi`, `dir` and `<bdi>` order,
  align and mirror text; long lines overflow and clip on the left; carets and
  selections follow the text.
- Every 2D transform function applies in the order written, and transitions
  between transforms interpolate as browsers do.
- `clip-path` clips elements to insets, circles and boxes;
  `backface-visibility: hidden` hides elements turned away from the viewer.
- Absolutely positioned marks inside lines of text, such as an editor's
  hard-break marks, sit beside the text they belong to.
- Bars and popups drawn over a scrolling area, such as a search bar over an
  editor, stay visible while the area scrolls.

## 0.1.0-alpha.1

First preview release.

- `nativeboat build` compiles an Electron project into a native Windows
  executable that runs without Chromium, WebView2, Node.js or a JavaScript
  engine.
- `nativeboat analyze` reports how well the native target supports a project.
- Works with plain Electron, electron-vite, vite-plugin-electron and Electron
  Forge (Vite) projects, TypeScript projects, and pnpm, npm and Yarn
  workspaces.
- Nothing else to install: no Rust, Visual Studio or Windows SDK.
- Executables carry the application's name, version and icon, and can be
  signed with `--sign`.
