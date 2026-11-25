# Builds a NativeBoat release package from this checkout. The release
# workflow (.github/workflows/release.yml) runs it for a version tag.
#
#   ./scripts/build-release.ps1
#
# 1. Packages the pinned Rust toolchain (target\release\toolchain).
# 2. Builds a development nativeboat.exe, which embeds the runtime's sources,
#    and with it precompiles the runtime libraries into
#    target\release\toolchain\runtime-libs.
# 3. Builds the released nativeboat.exe without the runtime's sources
#    (target\release), and checks that nothing in it or its toolchain is
#    source code.
# 4. Refreshes THIRD-PARTY-LICENSES.txt from the shipped dependencies.
# 5. Packages nativeboat-<version>-windows-x64.zip and SHA256SUMS.txt into
#    target\release-package\<version> with scripts\prepare-release.ps1, and
#    checks the package again.
#
# Nothing is tagged or uploaded. Run it with no other builds in progress.
param([switch]$NoBuild)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$manifest = Get-Content -Raw (Join-Path $root 'Cargo.toml')
if ($manifest -notmatch '(?m)^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"') {
    throw 'No workspace version in Cargo.toml'
}
$version = $Matches[1]
$release = Join-Path $root 'target\release'
$cargoManifest = Join-Path $root 'Cargo.toml'
$noSources = Join-Path $PSScriptRoot 'check-no-sources.py'

if (-not $NoBuild) {
    & (Join-Path $PSScriptRoot 'package-toolchain.ps1') -Destination (Join-Path $release 'toolchain')

    # The development build compiles the runtime from its embedded sources.
    # It runs from the release folder so it uses the packaged toolchain.
    cargo build --release --locked -p nativeboat-cli --manifest-path $cargoManifest
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $exporter = Join-Path $release 'nativeboat-export.exe'
    Copy-Item -LiteralPath (Join-Path $release 'nativeboat.exe') -Destination $exporter -Force
    try {
        & $exporter export-runtime (Join-Path $release 'toolchain\runtime-libs')
        if ($LASTEXITCODE -ne 0) { throw 'Exporting the runtime libraries failed' }
    } finally {
        Remove-Item -LiteralPath $exporter -Force -ErrorAction SilentlyContinue
    }

    # The released compiler carries no runtime sources.
    cargo build --release --locked -p nativeboat-cli --no-default-features --manifest-path $cargoManifest
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}

python $noSources (Join-Path $release 'nativeboat.exe') (Join-Path $release 'toolchain')
if ($LASTEXITCODE -ne 0) { throw 'The release build carries source code' }

python (Join-Path $PSScriptRoot 'third-party-licenses.py') (Join-Path $root 'THIRD-PARTY-LICENSES.txt')
if ($LASTEXITCODE -ne 0) { throw 'Third-party licenses need attention' }

& (Join-Path $PSScriptRoot 'prepare-release.ps1') -Version $version -BinaryDirectory $release `
    -OutputDirectory (Join-Path $root 'target\release-package')

python $noSources (Join-Path $root "target\release-package\$version\nativeboat-$version-windows-x64")
if ($LASTEXITCODE -ne 0) { throw 'The release package carries source code' }
