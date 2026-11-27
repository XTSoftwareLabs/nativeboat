# Runs a native VS Code build with its own user data and extensions folders,
# a folder and its README open, and settings that skip the start page and
# the AI sign-in, then saves a window snapshot, its output and VS Code's
# logs into -OutputDirectory. A task the folder runs when it opens shows the
# integrated terminal. -Environment sets variables such as
# NATIVEBOAT_TRACE_IPC=1 for the run, and -Settings is a JSON object of
# more settings, such as {"workbench.editorAssociations": {...}}.
param(
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Folder,
    [int]$WaitSeconds = 120,
    [string[]]$Environment = @(),
    [string]$Settings = ''
)
$ErrorActionPreference = 'Stop'
foreach ($setting in $Environment | Where-Object { $_ }) {
    $name, $value = $setting -split '=', 2
    Set-Item "env:$($name.Trim())" $value
}
$executable = Get-ChildItem $Out -Filter *.exe | Select-Object -First 1
if (-not $executable) { throw "No executable in $Out" }
$user = Join-Path $OutputDirectory 'data\User'
New-Item -ItemType Directory -Force $user | Out-Null
$values = @{
    'workbench.startupEditor' = 'none'
    'chat.disableAIFeatures' = $true
    'telemetry.telemetryLevel' = 'off'
    'update.mode' = 'none'
    'extensions.autoUpdate' = $false
    'workbench.enableExperiments' = $false
    'task.allowAutomaticTasks' = 'on'
}
if ($Settings) {
    foreach ($property in ($Settings | ConvertFrom-Json).PSObject.Properties) { $values[$property.Name] = $property.Value }
}
$values | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $user 'settings.json')
$tasks = Join-Path $Folder '.vscode'
New-Item -ItemType Directory -Force $tasks | Out-Null
@{
    version = '2.0.0'
    tasks = @(@{
        label = 'native terminal'
        type = 'shell'
        command = 'echo native terminal works; Get-Location'
        runOptions = @{ runOn = 'folderOpen' }
        presentation = @{ reveal = 'always' }
    })
} | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $tasks 'tasks.json')
$arguments = "--user-data-dir=$OutputDirectory\data", "--extensions-dir=$OutputDirectory\extensions",
    '--enable-smoke-test-driver', '--disable-workspace-trust', $Folder, (Join-Path $Folder 'README.md')
& (Join-Path $PSScriptRoot '..\app-snapshot.ps1') -Executable $executable.FullName -OutputDirectory $OutputDirectory `
    -WaitSeconds $WaitSeconds -Arguments $arguments
Get-Process | Where-Object { $_.Path -eq $executable.FullName } | Stop-Process -Force -ErrorAction SilentlyContinue
$logs = Join-Path $OutputDirectory 'data\logs'
if (Test-Path $logs) { Copy-Item $logs (Join-Path $OutputDirectory 'logs') -Recurse }
Remove-Item (Join-Path $OutputDirectory 'data'), (Join-Path $OutputDirectory 'extensions'), (Join-Path $OutputDirectory 'profile') `
    -Recurse -Force -ErrorAction SilentlyContinue
Get-ChildItem (Join-Path $OutputDirectory 'logs') -Recurse -Filter *.log -ErrorAction SilentlyContinue |
    Select-String -Pattern 'error|failed' -SimpleMatch:$false | Select-Object -First 60 | ForEach-Object { Write-Host $_ }
