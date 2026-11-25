# Runs tests with coverage instrumentation and reports production coverage.
# The 95% line target is best effort, not a completion or script-failure gate.
# Test code (tests.rs files and tests/ directories) is excluded from the report.
param(
    [switch]$Html,
    [string[]]$Package,
    [switch]$Clean
)

$ErrorActionPreference = 'Stop'
$ignore = '([\/]tests?[\/]|[\/]tests?\.rs$)'
# Every test binary runs, so one failing crate does not hide the others.
$common = @('--no-fail-fast', '--ignore-filename-regex', $ignore)
if ($Package) {
    foreach ($name in $Package) { $common += @('--package', $name) }
} else {
    $common += '--workspace'
}

$scope = 'workspace'
if ($Package) {
    $digest = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes((($Package | Sort-Object -Unique) -join "`n"))
        $scope = [BitConverter]::ToString($digest.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant().Substring(0, 16)
    } finally { $digest.Dispose() }
}
# Each scope has its own reusable cache: binaries from another package scope
# can contain older instrumented copies of this package's production code.
$coverageTarget = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../target/llvm-cov-$scope"))
$overrides = @{
    CARGO_INCREMENTAL = '0'
    CARGO_PROFILE_DEV_OPT_LEVEL = '0'
    CARGO_PROFILE_TEST_OPT_LEVEL = '0'
    CARGO_LLVM_COV_TARGET_DIR = $coverageTarget
}
$savedEnvironment = @{}
foreach ($name in $overrides.Keys) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    [Environment]::SetEnvironmentVariable($name, $overrides[$name], 'Process')
}

$result = 1
try {
    if ($Clean) {
        cargo llvm-cov clean --workspace
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    if ($Html) {
        cargo llvm-cov @common --html
    } else {
        cargo llvm-cov @common --show-missing-lines
    }
    $result = $LASTEXITCODE
} finally {
    try {
        if ($Clean) {
            # Keep exported HTML reports; discard only the instrumented build.
            cargo clean --target-dir $coverageTarget --profile dev
            if ($result -eq 0) { $result = $LASTEXITCODE }
        }
    } finally {
        foreach ($name in $savedEnvironment.Keys) {
            if ($null -eq $savedEnvironment[$name]) {
                Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
            } else {
                [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
            }
        }
    }
}
exit $result
