# Third-party notices

NativeBoat is licensed under the [MIT license](LICENSE). It includes and builds
with third-party software. Each component remains under its own license,
which NativeBoat's license does not change.

## In the NativeBoat download

- **Rust toolchain.** The `toolchain` folder contains unmodified official
  Rust 1.97.1 binaries and libraries that `nativeboat build` uses to compile
  and link applications: `rustc` with its libraries (which contain LLVM),
  `rust-lld` (LLVM's linker) and Rust's `core`, `alloc` and
  `compiler_builtins` libraries. The Rust Project Contributors license them
  under the MIT license or the Apache License 2.0; LLVM is under the Apache
  License 2.0 with LLVM Exceptions. Their copyright and license files are in
  `toolchain/share/doc/rust` (`COPYRIGHT.html`, `COPYRIGHT-library.html` and
  `licenses`).
  "Rust" is a trademark of the Rust Foundation. NativeBoat uses the Rust
  compiler and is not affiliated with or endorsed by the Rust Foundation.
- **Rust crates.** `nativeboat.exe` and the libraries in the `toolchain`
  folder contain open source Rust crates under permissive licenses (MIT,
  Apache-2.0, BSD, ISC, Zlib, Unicode and similar).
  [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt) lists each crate with
  its license texts.

## In applications built with NativeBoat

Applications built with NativeBoat contain the NativeBoat runtime (MIT) and
third-party code: Rust's standard libraries, math functions derived from musl
and FreeBSD, Unicode data, algorithms adapted from Node.js and V8, and
libraries for layout, regular expressions, URLs, text encodings, compression
and Windows APIs. Their copyright notices and licenses are embedded in every
executable, and each build writes them to `THIRD-PARTY-NOTICES.txt` next to
the executable.

When you distribute an application, include that `THIRD-PARTY-NOTICES.txt`
file with it, for example in its installation folder.

## In the source code

### Rust standard libraries and math functions

Executables built by NativeBoat contain Rust's `core`, `alloc` and
`compiler_builtins` and math functions vendored from the `libm` crate 0.2.16
(MIT; a port of musl and FreeBSD msun, derived from Sun fdlibm). Their notices
are embedded in every executable (`crates/nativeboat-start/src/NOTICES.txt`)
and retained in the vendored sources (`crates/nativeboat-native/src/libm.rs`).

Their string functions also contain Unicode 15.1 character data generated from
the Unicode Character Database (`scripts/generate-unicode-tables.py`), under the
Unicode License v3, whose notice is embedded likewise.

### Node.js and V8 algorithms

Native inspection and console formatting adapt Node.js inspection algorithms
(MIT, Copyright Node.js contributors). Number-to-radix formatting follows a
port of V8's algorithm (BSD three-clause, Copyright 2014, the V8 project
authors). Their notices are embedded in every generated executable in
`crates/nativeboat-start/src/NOTICES.txt`. These are native implementations;
generated executables contain neither Node.js nor the V8 engine.

Upstream license sources: [Node.js](https://github.com/nodejs/node/blob/v22.15.0/LICENSE)
and [V8](https://github.com/v8/v8/blob/main/LICENSE).

`crates/nativeboat-cli/node` contains Node.js v22.15.0 library modules (MIT,
Copyright Node.js contributors and Joyent, Inc.), including Node's bundled
minimatch (ISC, Copyright Isaac Z. Schlueter and Contributors) with
balanced-match and brace-expansion (MIT, Copyright Julian Gruber). NativeBoat
compiles them into applications that use them, and their notices are
embedded likewise.

### Windows keyboard mapping tables

The Windows keyboard mapping tables in `crates/nativeboat-native/src/keyboard/data.rs`
derive from Microsoft's native-keymap 3.3.9 and its Chromium DOM code table.

- Microsoft Corporation: [MIT license](crates/nativeboat-native/src/keyboard/LICENSE-native-keymap).
- Copyright 2015 The Chromium Authors: [BSD license](crates/nativeboat-native/src/keyboard/LICENSE-chromium).

Retain these licenses and notices when redistributing the tables or binaries
containing them.

### Regular expression matcher

`crates/nativeboat-regexp-build/vendor` retains unmodified registry source and
license files for regress 0.12.0, hashbrown 0.17.1, allocator-api2 0.2.21 and
equivalent 1.0.2 (MIT or Apache-2.0), memchr 2.8.3 (MIT or Unlicense), and
foldhash 0.2.0 (Zlib, Copyright 2024 Orson Peters). The selected MIT and Zlib
notices are exposed as `nativeboat_regexp_build::NOTICES`. `nativeboat build`
links the matcher into every generated executable and embeds these notices in
it. Package archive and file digests are in `vendor/PINS.json` and
`vendor/SHA256SUMS`.

`crates/nativeboat-regexp-build/overlays/regress` replaces two regress modules
when building: `api.rs` makes compiled capture-name metadata public (the
matching algorithms are unchanged), and `unicodetables.rs` is regenerated
from Unicode 16.0.0 data by regress's own pinned generator, so property
escapes agree with Node 22. The data is used under the Unicode License v3,
appended to `nativeboat_regexp_build::NOTICES`. Generator commit, input and
output digests and the two local generator edits are in
`overlays/regress/UNICODE-PINS.json`.

### URL parser and text encodings

`crates/nativeboat-url-build/vendor` retains rust-url 2.5.8, idna 1.1.0 and
their dependencies (MIT or Apache-2.0; unicode-joining-type under Apache-2.0;
tinyvec also under Zlib; Unicode data under the Unicode License v3), and
`crates/nativeboat-encoding-build/vendor` retains chardetng 1.0.0,
encoding_rs 0.8.35 (MIT or Apache-2.0, with WHATWG data under a BSD license),
cfg-if 1.0.5 and memchr 2.8.3, each with its original license files. Their
notices are exposed as the build crates' `NOTICES` and embedded in every
generated executable that links them.

### DEFLATE decoder

`crates/nativeboat-cli/vendor` retains the unmodified no-std source files for
miniz_oxide 0.9.1 (MIT, Apache-2.0 or Zlib) and its adler2 2.0.1 dependency
(MIT, Apache-2.0 or 0BSD). The native application runtime uses the raw and
zlib inflate implementations; gzip framing is handled by NativeBoat. Original
license files are retained beside each vendored source tree.
