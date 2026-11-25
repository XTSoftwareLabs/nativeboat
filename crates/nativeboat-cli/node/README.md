# Node.js library sources

Node.js v22.15.0 library modules compiled into the `nativeboat_node`
library that applications link when they use them.

These `.js` files are embedded compiler inputs, parsed by nativeboat's Rust
frontend and lowered to native code. Neither nativeboat nor its generated
applications needs Node to execute them; applications link compiled machine
code, without a Node process, interpreter or executable JavaScript bundle.
Node runs local reference comparisons. Installing an application's npm
packages is a separate preparation step; an installed source tree compiles
without Node.

- Unchanged copies: `lib/path.js`, `lib/path/*.js`, `lib/buffer.js`,
  `lib/events.js`, `lib/os.js`, `lib/process.js`, `lib/querystring.js`,
  `lib/url.js`, `lib/internal/url.js`,
  `lib/internal/buffer.js`, `lib/internal/constants.js`,
  `lib/internal/querystring.js`, `lib/internal/assert.js`,
  `lib/internal/fs/utils.js`,
  `lib/fs.js`, `lib/fs/promises.js`, `lib/internal/fs/promises.js`,
  `lib/internal/fs/read/context.js`, `lib/internal/blob.js`,
  `lib/internal/streams/destroy.js`,
  `lib/internal/process/permission.js`,
  `lib/internal/util/types.js`,
  `lib/internal/per_context/domexception.js` and
  `lib/internal/deps/minimatch/index.js` (Node's `deps/minimatch/index.js`).
- Unchanged readline sources: `lib/readline.js`, `lib/readline/promises.js`,
  `lib/internal/readline/*.js`, and `lib/internal/net.js`.
- `lib/http2.js` and `lib/tls.js` are written for the native runtime:
  Node's constants and classes, with sessions, sockets and servers that
  report they are not available.
- Web streams: `lib/stream/web.js` and `lib/internal/webstreams/*.js` are
  unchanged copies, except that `readablestream.js`, `writablestream.js` and
  `transformstream.js` take `DOMException` from the bundled
  `internal/per_context/domexception` instead of the messaging binding.
  `lib/internal/worker/io.js` is written for the runtime: applications have
  no message ports, so constructing a `MessageChannel` throws.
- `lib/assert.js` is written for the runtime with Node's API and the
  bundled deep equality; messages Node builds from the caller's source show
  the values instead, and CallTracker is not available.
- `lib/internal/fetch.js` is written for the runtime: the Fetch API globals
  (`fetch`, `Headers`, `Request`, `Response`, `FormData`) over the `http`
  and `https` modules; Node's undici is not bundled.
- `lib/internal/electron/net.js` is written for the runtime: Electron's
  `net.request` over the `http` and `https` modules.
- `lib/net.js`, `lib/http.js` and `lib/https.js` are written for the native
  runtime with Node's classes, events and errors: TCP sockets and servers
  on the `tcp` binding (Winsock), HTTP/1.1 servers and clients over them,
  and HTTPS requests on the native client (`http_client`, WinHTTP).
- Unchanged util sources: `lib/util.js`,
  `lib/internal/util/debuglog.js` and `lib/internal/util/comparisons.js`.
- Subsets of Node's files with definitions copied unchanged:
  `lib/internal/validators.js`, `lib/internal/process/per_thread.js`,
  `lib/internal/worker/js_transferable.js` (transfer marking, its own messaging symbols and the runtime's `structuredClone`),
  `lib/internal/util.js` (whose
  `lazyDOMException` takes `DOMException` from the bundled module instead of
  the messaging binding, and whose private RegExp bootstrap uses protected
  native intrinsics instead of a VM string evaluation).
- `lib/internal/primordials.js` makes the primordials these modules use.
- `lib/timers.js` and `lib/timers/promises.js` export Node's `timers` and
  `timers/promises` APIs over the timer functions of the native runtime,
  which schedules timers itself (Node's own sources build on its timer
  lists and libuv bindings).

Modules receive `primordials` and `internalBinding` as Node's internal
module wrapper supplies them. `internal/errors`, `internal/options`,
`internal/util/inspect` (inspection, formatting and terminal-control stripping)
and the bindings (`buffer`, `config`, `icu`, `trace_events`,
`string_decoder`, `types`, `util`, `url`, `encoding_binding`, `os`,
`constants`, `credentials`) are
provided by the native runtime. The compiler maps public `util`, `fs` and
`fs/promises` imports into their Node source graph, including the native
`fs_event_wrap` binding. A focused generated executable exercises directory
watcher creation/close, path and descriptor reads, `fs.promises.readFile`, and
a file-change callback with an empty `PATH`. The generated run produced all
expected output. The assertion compares output without assuming an order
between watcher and promise callbacks. This is targeted operation coverage, not full
Node filesystem compatibility or MarkText acceptance. Native file-target
event filtering also has focused tests. Internal lazy imports not
implemented yet also throw if reached.

The unchanged public `util` dependency graph now passes source compilation.
Native formatter, depth-option, terminal-control and monotonic-time helpers
have focused tests. Generated `util` module initialization and application use
remain unverified; compiling the source graph alone is not a public runtime
acceptance receipt.

The native `crypto` module provides incremental `createHash`, secure
`randomBytes` and `randomUUID` using Windows CNG and the OS random source. A
generated executable checks streamed SHA-1/SHA-256 digests, returned Buffer
behavior and UUID shape with an empty `PATH`. This is the observed MarkText
hash/random subset; optional `conf` encryption and the wider Node crypto API
remain unsupported.

Node.js is copyright Node.js contributors and Joyent, Inc., under the MIT
license; minimatch is copyright Isaac Z. Schlueter and Contributors under the
ISC license; its bundled balanced-match and brace-expansion are copyright
Julian Gruber under the MIT license. The full texts are in
`crates/nativeboat-start/src/NOTICES.txt`, which every built executable
embeds.
