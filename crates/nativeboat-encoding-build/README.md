# Native encoding detector sources

This build helper packages `chardetng` 1.0.0 and its no-std dependencies so
nativeboat can build them with the selected Rust toolchain without Cargo,
registry access, a C++ compiler, the C runtime, or a JavaScript engine at app
build time. `encoding_rs` is pinned to 0.8.35 for this freestanding build; the
optional SIMD dispatch is omitted. The detector's output labels are mapped to
the CED names consumed by MarkText's existing encoding conversion code.

Vendored source versions and notices are kept in `vendor/`; source inclusion
and licenses are embedded in the generated application's notices.
