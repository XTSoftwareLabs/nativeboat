# Packaged native matcher sources

This build-time crate embeds the unmodified source, manifests and license files
of six pinned registry packages. `vendor/PINS.json` records package archive
SHA-256 digests and original VCS commits; `vendor/SHA256SUMS` records each copied
file's SHA-256 digest. Vendor files retain their original bytes and licenses.
The matcher wrapper is embedded from `../nativeboat-regexp/src/lib.rs`.

`build` writes these sources to an owned staging directory, then asks the
caller-selected rustc/sysroot to compile them in dependency order. It does not
invoke Cargo, a registry, build scripts, MSVC or the Windows SDK. It rebuilds
the libraries on each call; `nativeboat build` caches the result per toolchain.
The caller must link with the same target/sysroot and add the returned rlib and
dependency search directory. It must append `NOTICES` to the notices retained
in the generated executable, as the executable test does.

This crate uses Rust std only during compilation. The matcher wrapper and its
six dependencies compile with std disabled, linking to core/alloc and native
process startup. Upstream lint levels are capped as Cargo caps dependency
lints; the nativeboat wrapper is built with warnings denied.

`nativeboat build` uses it to link the matcher into every generated
executable, with these notices. It contains no JavaScript parser or execution engine;
regress parses regular expression pattern data and matches text using native
backtracking code. RegExp objects, literals and protocols live in the native
runtime (`regexp_builtin.rs`). Bounded worst-case matching time is not
established, and neither matcher tests nor Rust coverage establish full
MarkText compatibility or startup gains.

`overlays/regress` replaces three regress modules. `classicalbacktrack.rs`
lets start-anchored patterns (`^` without the `m` flag) try only the current
position for UTF-16 input too; upstream skips that check whenever its
`utf16` feature is on, so every failed `^` match scanned the whole input. `api.rs` exposes compiled
capture-group names (matching is unchanged), and `unicodetables.rs` is
regenerated from Unicode 16.0.0 by regress's pinned generator, because the
upstream Unicode 17 tables (for example the Beria Erfe script) disagree with
Node 22.15.0. `UNICODE-PINS.json` records the generator commit, input, output
and local-edit digests. The workspace builds the same overlay as a path
dependency.
