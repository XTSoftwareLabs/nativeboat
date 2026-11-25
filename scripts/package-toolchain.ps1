# Copies the pinned Rust compiler, linker and target libraries that
# `nativeboat build` uses into `toolchain` next to nativeboat.exe, so users
# install no Rust, MSVC or Windows SDK. Only what native builds need is
# copied: rustc and its libraries, rust-lld, and the core/alloc/
# compiler_builtins libraries of the x86_64-pc-windows-msvc target, with
# Rust's license notices. All of these binaries import only Windows system
# DLLs; the official builds link their C runtime statically.
#
#   ./scripts/package-toolchain.ps1                         # next to target\release\nativeboat.exe
#   ./scripts/package-toolchain.ps1 -Destination dist\toolchain
param(
    [string]$Destination = (Join-Path $PSScriptRoot '..\target\release\toolchain'),
    [string]$Rustc = 'rustc'
)

$ErrorActionPreference = 'Stop'
$pinned = (Get-Content (Join-Path $PSScriptRoot 'toolchain-version.txt') -Raw).Trim()
$version = (& $Rustc -V).Trim()
if ($version -notlike "rustc $pinned *") {
    throw "nativeboat ships Rust $pinned, but $Rustc is '$version'"
}
$sysroot = (& $Rustc --print sysroot).Trim()
$target = 'lib\rustlib\x86_64-pc-windows-msvc'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$files = @(
    'bin\rustc.exe'
    (Get-ChildItem (Join-Path $sysroot 'bin') -Filter 'rustc_driver-*.dll').Name | ForEach-Object { "bin\$_" }
    (Get-ChildItem (Join-Path $sysroot 'bin') -Filter 'std-*.dll').Name | ForEach-Object { "bin\$_" }
    "$target\bin\rust-lld.exe"
    foreach ($crate in 'core', 'alloc', 'compiler_builtins') {
        (Get-ChildItem (Join-Path $sysroot "$target\lib") -Filter "lib$crate-*").Name |
            ForEach-Object { "$target\lib\$_" }
    }
    'share\doc\rust\COPYRIGHT.html'
    'share\doc\rust\COPYRIGHT-library.html'
)
$licenses = Join-Path $sysroot 'share\doc\rust\licenses'

if (Test-Path $Destination) { Remove-Item -Recurse -Force $Destination }
foreach ($file in $files) {
    $to = Join-Path $Destination $file
    New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
    Copy-Item (Join-Path $sysroot $file) $to
}
Copy-Item -Recurse $licenses (Join-Path $Destination 'share\doc\rust\licenses')
& $Rustc -vV | Out-File -Encoding utf8 (Join-Path $Destination 'VERSION')

# Build the renderer backend once during NativeBoat packaging. Generated app
# builds use rustc only; these target-specific rlibs travel beside the
# compiler and are copied into its fingerprinted runtime cache on demand.
$targetTriple = 'x86_64-pc-windows-msvc'
if ($env:RUSTC) { $savedRustc = $env:RUSTC }
try {
    $env:RUSTC = $Rustc
    # Cargo's artifact messages name the exact files this build linked; the
    # deps directories can also hold other builds of the same crates.
    $messages = & cargo build --manifest-path (Join-Path $workspace 'Cargo.toml') `
        --package nativeboat-layout --package nativeboat-win32 --no-default-features --release `
        --target $targetTriple --locked --message-format=json-render-diagnostics
} finally {
    if ($savedRustc) { $env:RUSTC = $savedRustc } else { Remove-Item Env:\RUSTC -ErrorAction SilentlyContinue }
}
if ($LASTEXITCODE -ne 0) { throw 'Failed to build the no-std renderer libraries' }
$artifacts = @{}
foreach ($line in $messages) {
    if (-not $line.StartsWith('{')) { continue }
    $message = $line | ConvertFrom-Json
    if ($message.reason -ne 'compiler-artifact') { continue }
    foreach ($file in $message.filenames) {
        $artifacts[$message.target.name.Replace('-', '_') + [IO.Path]::GetExtension($file)] = $file
    }
}

$dependencies = @(
    'nativeboat_win32', 'windows_sys', 'windows_link',
    'nativeboat_layout', 'nativeboat_ir', 'nativeboat_dom', 'nativeboat_css',
    'nativeboat_html', 'taffy', 'slotmap', 'smallvec', 'arrayvec', 'libm',
    'serde', 'serde_core'
)
$rendererLibraries = Join-Path $Destination 'renderer-libs'
New-Item -ItemType Directory -Force $rendererLibraries | Out-Null
foreach ($crate in $dependencies) {
    $rlib = $artifacts["$crate.rlib"]
    if (-not $rlib) { throw "The renderer build produced no target rlib for $crate" }
    if (-not ([IO.Path]::GetFileName($rlib)).StartsWith("lib$crate-")) {
        # The built package is reported by its uplifted copy; generated builds
        # need the hashed file in deps, which has the same contents.
        $hash = (Get-FileHash -LiteralPath $rlib).Hash
        $rlib = (Get-ChildItem -LiteralPath (Join-Path (Split-Path $rlib) 'deps') -Filter "lib$crate-*.rlib" |
            Where-Object { (Get-FileHash -LiteralPath $_.FullName).Hash -eq $hash } |
            Select-Object -First 1).FullName
        if (-not $rlib) { throw "No hashed target rlib in deps matches the built $crate" }
    }
    Copy-Item -LiteralPath $rlib -Destination $rendererLibraries
}
$derive = $artifacts['serde_derive.dll']
if (-not $derive) { throw 'The renderer build produced no host serde_derive proc-macro library' }
Copy-Item -LiteralPath $derive -Destination $rendererLibraries

$size = (Get-ChildItem $Destination -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ('Packaged {0} into {1} ({2:N0} MB)' -f $version, (Resolve-Path $Destination), $size)
