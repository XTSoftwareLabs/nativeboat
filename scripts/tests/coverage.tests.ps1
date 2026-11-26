# Exercise coverage orchestration with fake Cargo; no build or cleanup runs.
$ErrorActionPreference = 'Stop'
$gate = Join-Path $PSScriptRoot '../coverage.ps1'
$names = @('CARGO_INCREMENTAL', 'CARGO_PROFILE_DEV_OPT_LEVEL', 'CARGO_PROFILE_TEST_OPT_LEVEL', 'CARGO_LLVM_COV_TARGET_DIR')
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
function global:cargo {
    param([Parameter(ValueFromRemainingArguments)][object[]]$Arguments)
    foreach ($name in $names) {
        if ($name -eq 'CARGO_LLVM_COV_TARGET_DIR') {
            if ($env:CARGO_LLVM_COV_TARGET_DIR -notmatch '[\\/]target[\\/]llvm-cov-([a-f0-9]{16}|workspace)$') {
                throw 'Coverage did not isolate the package scope'
            }
            $global:coverageTestTargets += $env:CARGO_LLVM_COV_TARGET_DIR
            continue
        }
        if ([Environment]::GetEnvironmentVariable($name, 'Process') -ne '0') {
            throw "Coverage did not isolate $name"
        }
    }
    if ($Arguments -notcontains 'llvm-cov' -or $Arguments -notcontains '--ignore-filename-regex') {
        throw 'Expected coverage that leaves out test files'
    }
    if ($global:coverageTestExit -eq 'throw') { throw 'synthetic Cargo exception' }
    $global:LASTEXITCODE = $global:coverageTestExit
}
try {
    $global:coverageTestTargets = @()
    foreach ($ambient in @($null, '2')) {
        foreach ($name in $names) {
            if ($null -eq $ambient) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
            else { [Environment]::SetEnvironmentVariable($name, $ambient, 'Process') }
        }
        foreach ($code in @(0, 101)) {
            $global:coverageTestExit = $code
            & $gate -Package nativeboat-dom
            if ($LASTEXITCODE -ne $code) { throw 'Coverage result was lost' }
            foreach ($name in $names) {
                if ([Environment]::GetEnvironmentVariable($name, 'Process') -cne $ambient) {
                    throw "Coverage did not restore $name"
                }
            }
        }
        $global:coverageTestExit = 'throw'
        $failed = $false
        try { & $gate -Package nativeboat-dom }
        catch {
            if ($_.Exception.Message -ne 'synthetic Cargo exception') { throw }
            $failed = $true
        }
        if (!$failed) { throw 'Expected the Cargo exception to propagate' }
        foreach ($name in $names) {
            if ([Environment]::GetEnvironmentVariable($name, 'Process') -cne $ambient) {
                throw "Coverage did not restore $name after an exception"
            }
        }
    }
    $global:coverageTestExit = 0
    & $gate -Package @('nativeboat-dom', 'nativeboat-web')
    & $gate -Package @('nativeboat-web', 'nativeboat-dom', 'nativeboat-dom')
    $targets = $global:coverageTestTargets
    if ($targets[-1] -ne $targets[-2] -or $targets[-1] -eq $targets[0]) {
        throw 'Coverage package scopes must be stable and distinct'
    }
    & $gate
    if (!$global:coverageTestTargets[-1].EndsWith('llvm-cov-workspace')) {
        throw 'Workspace coverage must have its own scope'
    }
    Write-Output 'Coverage profile and environment checks passed (fake Cargo).'
    $global:LASTEXITCODE = 0
} finally {
    Remove-Item Function:global:cargo
    Remove-Variable coverageTestExit -Scope Global -ErrorAction SilentlyContinue
    Remove-Variable coverageTestTargets -Scope Global -ErrorAction SilentlyContinue
    foreach ($name in $names) {
        if ($null -eq $saved[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    }
}
