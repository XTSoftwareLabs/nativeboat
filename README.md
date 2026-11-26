# NativeBoat

[![CI](https://github.com/XTSoftwareLabs/nativeboat/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/XTSoftwareLabs/nativeboat/actions/workflows/ci.yml)
[![Release](https://github.com/XTSoftwareLabs/nativeboat/actions/workflows/release.yml/badge.svg)](https://github.com/XTSoftwareLabs/nativeboat/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/XTSoftwareLabs/nativeboat?include_prereleases&sort=semver)](https://github.com/XTSoftwareLabs/nativeboat/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platform: Windows x64](https://img.shields.io/badge/platform-Windows%20x64-0078d4.svg)
![Rust 1.96](https://img.shields.io/badge/rust-1.96-orange.svg)

NativeBoat compiles an Electron application into a native Windows executable.
It takes your project as it is: its JavaScript or TypeScript, HTML, CSS and
installed dependencies. The executable it builds runs without Chromium,
WebView2, Node.js or a JavaScript engine.

You don't change your project, and you can keep building the Electron version
from the same repository.

> NativeBoat is in alpha. Check [Compatibility](docs/compatibility.md) for
> what works today, and run `nativeboat analyze` on your project before you
> build it.

## Download and install

1. Download `nativeboat-<version>-windows-x64.zip` from the
   [releases page](https://github.com/XTSoftwareLabs/nativeboat/releases).
2. Extract it into a folder such as `C:\Tools\nativeboat`.
3. Add that folder to your user `PATH` and open a new terminal.

Check the installation:

```powershell
nativeboat --version
```

### Requirements

- Windows 10 version 1607 or later, or Windows 11, on x64. Compiled
  applications have the same requirement.
- Node.js and your project's package manager, to install the project's
  dependencies.
- Any tools your project's own build scripts use.

You don't need Visual Studio, the Windows SDK or a Rust installation: the
download brings everything NativeBoat needs to build an application.

## Compile an application

From your Electron project's folder:

```powershell
npm install             # or pnpm install, yarn: as for an Electron build
nativeboat analyze      # report what the project uses and how well it is supported
nativeboat build        # writes dist\<productName>.exe
dist\<productName>.exe
```

## Documentation

- [Getting started](docs/getting-started.md): installing, building, signing,
  installers and automatic updates.
- [Compatibility](docs/compatibility.md): the Electron, Node.js and web
  features NativeBoat supports.
- [How NativeBoat works](docs/how-it-works.md): how an Electron app becomes
  a native program.
- [Diagnostics](docs/diagnostics.md): tracing and profiling a compiled
  application.
- [Changelog](CHANGELOG.md).

## Build from source

NativeBoat is written in Rust. See [CONTRIBUTING.md](CONTRIBUTING.md) to build
it, run its tests and make a release package.

## License

NativeBoat is open source under the [MIT license](LICENSE).

It includes and builds with third-party software that keeps its own licenses;
see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). Every application
NativeBoat builds embeds the notices of the code it contains, and the build
writes them to `THIRD-PARTY-NOTICES.txt` next to the executable: include that
file when you distribute the application.
