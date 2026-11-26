# Builds VS Code natively for the VS Code workflow. Stops after -Minutes so
# that a later job can resume from the generated-crate cache, and writes
# `state=built|partial|failed` to $env:GITHUB_OUTPUT when it is set.
param(
    [Parameter(Mandatory)][string]$Nativeboat,
    [Parameter(Mandatory)][string]$Repository,
    [Parameter(Mandatory)][string]$Work,
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$Log,
    [int]$Minutes = 290
)
$ErrorActionPreference = 'Stop'
$env:NATIVEBOAT_KEEP_GOING = '1'
$arguments = 'build', $Repository, '--config', (Join-Path $PSScriptRoot 'vscode.json'), '--out', $Out, '--work-dir', $Work
$process = Start-Process -FilePath $Nativeboat -ArgumentList $arguments -NoNewWindow -PassThru `
    -RedirectStandardOutput "$Log.out" -RedirectStandardError "$Log.err"
$null = $process.Handle
if ($process.WaitForExit($Minutes * 60 * 1000)) {
    $state = if ($process.ExitCode -eq 0) { 'built' } else { 'failed' }
} else {
    taskkill /F /T /PID $process.Id | Out-Null
    $process.WaitForExit()
    $state = 'partial'
}
Get-Content "$Log.err", "$Log.out" | Set-Content $Log
Remove-Item "$Log.err", "$Log.out"
$lines = Get-Content $Log
$compiled = @($lines | Select-String -SimpleMatch '(compiled)').Count
$hits = @($lines | Select-String -SimpleMatch '(cache hit)').Count
Write-Host "state $state, $compiled crates compiled, $hits from the cache"
$lines | Select-Object -Last 40 | Write-Host
if ($env:GITHUB_OUTPUT) { "state=$state" >> $env:GITHUB_OUTPUT }
