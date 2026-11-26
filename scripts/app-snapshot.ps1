# Launches a built application with an isolated profile, waits, saves a
# window screenshot and the process output, then ends the process. Also
# writes processes.txt: the CPU time of each process the executable runs
# as and of its busiest threads, with its command line.
param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [double]$WaitSeconds = 8,
    [string[]]$Arguments = @()
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class Snapshot {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
'@
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$profileDir = Join-Path $OutputDirectory 'profile'
New-Item -ItemType Directory -Force "$profileDir\Roaming", "$profileDir\Local" | Out-Null
$env:APPDATA = "$profileDir\Roaming"
$env:LOCALAPPDATA = "$profileDir\Local"
$start = @{ FilePath = $Executable; RedirectStandardOutput = "$OutputDirectory\stdout.log"; RedirectStandardError = "$OutputDirectory\stderr.log"; PassThru = $true }
if ($Arguments.Count) { $start.ArgumentList = $Arguments }
$process = Start-Process @start
Start-Sleep -Milliseconds ([int]($WaitSeconds * 1000))
$process.Refresh()
if ($process.HasExited) {
    "exited with $($process.ExitCode)"
} else {
    $handle = $process.MainWindowHandle
    $rect = New-Object Snapshot+RECT
    [void][Snapshot]::GetWindowRect($handle, [ref]$rect)
    $width = [Math]::Max(1, $rect.Right - $rect.Left); $height = [Math]::Max(1, $rect.Bottom - $rect.Top)
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $dc = $graphics.GetHdc(); [void][Snapshot]::PrintWindow($handle, $dc, 2); $graphics.ReleaseHdc($dc)
    $bitmap.Save("$OutputDirectory\window.png")
    "window '$($process.MainWindowTitle)' ${width}x${height} working set $([int]($process.WorkingSet64 / 1MB)) MB"
    $report = foreach ($running in Get-Process | Where-Object { $_.Path -eq $process.Path }) {
        $command = (Get-CimInstance Win32_Process -Filter "ProcessId=$($running.Id)").CommandLine
        '{0} cpu {1:n1} s, {2} MB, {3} threads: {4}' -f $running.Id, $running.TotalProcessorTime.TotalSeconds,
            [int]($running.WorkingSet64 / 1MB), $running.Threads.Count, $command
        $running.Threads | Sort-Object TotalProcessorTime -Descending | Select-Object -First 5 |
            ForEach-Object { '  thread {0} cpu {1:n1} s' -f $_.Id, $_.TotalProcessorTime.TotalSeconds }
    }
    $report | Set-Content "$OutputDirectory\processes.txt"
    Stop-Process -Id $process.Id -Force
}
