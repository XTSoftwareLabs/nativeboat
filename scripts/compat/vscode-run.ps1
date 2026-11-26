# Runs a native VS Code build with its own user data and extensions folders
# and a folder open, saves a window snapshot, its output and VS Code's logs
# into -OutputDirectory.
param(
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Folder,
    [int]$WaitSeconds = 120
)
$ErrorActionPreference = 'Stop'
$executable = Get-ChildItem $Out -Filter *.exe | Select-Object -First 1
if (-not $executable) { throw "No executable in $Out" }
$arguments = "--user-data-dir=$OutputDirectory\data", "--extensions-dir=$OutputDirectory\extensions",
    '--enable-smoke-test-driver', '--disable-workspace-trust', $Folder
& (Join-Path $PSScriptRoot '..\app-snapshot.ps1') -Executable $executable.FullName -OutputDirectory $OutputDirectory `
    -WaitSeconds $WaitSeconds -Arguments $arguments
Get-Process | Where-Object { $_.Path -eq $executable.FullName } | Stop-Process -Force -ErrorAction SilentlyContinue
$logs = Join-Path $OutputDirectory 'data\logs'
if (Test-Path $logs) { Copy-Item $logs (Join-Path $OutputDirectory 'logs') -Recurse }
Remove-Item (Join-Path $OutputDirectory 'data'), (Join-Path $OutputDirectory 'extensions'), (Join-Path $OutputDirectory 'profile') `
    -Recurse -Force -ErrorAction SilentlyContinue
Get-ChildItem (Join-Path $OutputDirectory 'logs') -Recurse -Filter *.log -ErrorAction SilentlyContinue |
    Select-String -Pattern 'error|failed' -SimpleMatch:$false | Select-Object -First 60 | ForEach-Object { Write-Host $_ }
