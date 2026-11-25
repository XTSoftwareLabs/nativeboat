# Packaged native URL parser

Pinned rust-url 2.5.8, idna 1.1.0 and the unicode-rs backend compile as
no_std native libraries. The adapter is pinned to 1.1.0; normalization and
joining tables use Unicode 16. Cargo, registry access, build scripts and
procedural macros are absent from application builds. Upstream package
archives and source files have SHA-256 pins. Full permissive license texts
are embedded through NOTICES (MIT, Apache-2.0 and Unicode-3.0 data terms).

The upstream unicode-joining-type generator runs only when preparing pinned
sources. Pre-generated tables and two include-path overlays are packaged;
the original vendor files remain unchanged. Application builds use
nativeboat's selected private Rust toolchain, core and alloc, without Node,
a JavaScript engine, a C runtime or an external Windows SDK.
