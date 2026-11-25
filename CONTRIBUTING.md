# Contributing to NativeBoat

Bug reports, compatibility reports and pull requests are welcome. When an
application does not build or behaves differently from Electron, please
include the `nativeboat analyze` output or the build error, the Electron
version and, if you can, a small project that shows the problem. Report
security problems privately instead, as [SECURITY.md](SECURITY.md) describes.

Fixes belong in the compiler, runtime or renderer and should work for every
application, not for one project.

## Building NativeBoat

NativeBoat is a Rust workspace for Windows x64. You need:

- Rust with the default MSVC target and the Visual C++ build tools that
  target uses. The workspace builds with Rust 1.96 or later, but the
  toolchain that NativeBoat ships, and builds applications with, is pinned in
  `scripts/toolchain-version.txt`, so install that version
  (`rustup default 1.97.1`). Applications that NativeBoat builds need no
  Visual C++ tools.
- Node.js 22, for the example application and for comparing behavior with
  Electron.
- Python 3, for the generator and packaging scripts in `scripts/`.

Build the command-line tool and give it a toolchain to build applications
with:

```powershell
cargo build -p nativeboat-cli
./scripts/package-toolchain.ps1 -Destination target\debug\toolchain
target\debug\nativeboat.exe --version
```

`package-toolchain.ps1` copies the installed Rust compiler and linker, which
must be the pinned version, next to `nativeboat.exe`, as a release package
has them.

## Tests and checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/coverage.ps1 -Package nativeboat-compiler    # line coverage of one crate
```

The workspace is large. On machines with little memory, limit parallel work
with `$env:CARGO_BUILD_JOBS = '1'` and run the CLI tests with
`cargo test -p nativeboat-cli -- --test-threads=1`, since each of them builds
an application.

Tests mock operating system and standard library calls with
[shimforge](https://crates.io/crates/shimforge) rather than test-only
abstractions. New code comes with tests that cover it, including its error
paths. CI (`.github/workflows/ci.yml`) runs formatting, clippy and the tests
with coverage, and builds the example application in `examples/react-vite`
to check executable size, startup time and memory against Electron.

## Repository layout

| Crate | Role |
| --- | --- |
| `nativeboat-cli` | the `nativeboat` command: project discovery, build steps, compilation, linking, installers |
| `nativeboat-project`, `nativeboat-bundler`, `nativeboat-analyzer` | project layouts and configuration, module resolution, `nativeboat analyze` |
| `nativeboat-compiler` | JavaScript and TypeScript to Rust compilation |
| `nativeboat-native` | the runtime that compiled code links: values, objects, built-ins, Node.js and Electron APIs |
| `nativeboat-page`, `nativeboat-dom`, `nativeboat-html`, `nativeboat-css`, `nativeboat-layout`, `nativeboat-win32`, `nativeboat-ir` | documents, styles, layout, painting and Win32 windows |
| `nativeboat-web` | the web platform layer (`src/js`) compiled into every application |
| `nativeboat-regexp*`, `nativeboat-url-build`, `nativeboat-encoding-build` | regular expressions, URLs and text encodings, with their vendored sources |
| `nativeboat-start`, `nativeboat-std-start` | executable entry points and embedded notices |
| `nativeboat-js`, `nativeboat-renderer` | a test harness that runs the web platform layer for unit tests |

`scripts/` holds the toolchain packaging, release, coverage and comparison
scripts and the generators of the Unicode, number and image test tables.
[How NativeBoat works](docs/how-it-works.md) gives an overview of the design.

## Third-party code

Code adapted or vendored from other projects keeps its license and notices.
Record it in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md); code that ends
up in built applications also needs its notice in
`crates/nativeboat-start/src/NOTICES.txt` (or in the `NOTICES` of the build
crate that packages it). New Cargo dependencies must use permissive
licenses: `scripts/third-party-licenses.py` fails the release build otherwise.

## Pull requests

Keep a pull request to one change, describe what it changes for
applications, and add an entry to the topmost section of
[CHANGELOG.md](CHANGELOG.md) for anything users notice. Contributions are
licensed under the [MIT license](LICENSE), the license of the project.

## Making a release

1. Set the version in `Cargo.toml` (`[workspace.package]`) and give
   `CHANGELOG.md` a section for it (`## <version>`).
2. With no other builds running, run:

   ```powershell
   ./scripts/build-release.ps1
   ```

   It packages the toolchain, precompiles the runtime libraries, builds the
   release `nativeboat.exe`, refreshes `THIRD-PARTY-LICENSES.txt`, and writes
   `nativeboat-<version>-windows-x64.zip` and `SHA256SUMS.txt` into
   `target\release-package\<version>`. `-NoBuild` repackages existing
   binaries; move an earlier package folder of the same version aside first.
3. Extract the ZIP into a fresh folder outside the system temp folder, run
   `nativeboat --version`, and build and run a representative Electron
   project with it.
4. Commit the refreshed license file, tag the release commit `v<version>`,
   and publish the ZIP and `SHA256SUMS.txt` as a GitHub release of this
   repository, marked as a pre-release for alpha and beta versions.
