# Startup and memory comparison for desktop applications, such as a native
# NativeBoat build and the original Electron package of the same application.
# For each run: launch, record the time until a visible, titled top-level
# window of the process tree appears, then the time until the tree's CPU use
# settles; record working set and private memory summed over the tree once
# settled; then end the tree. Reports medians per executable as JSON.
param(
    [Parameter(Mandatory)][string[]]$Executable,
    [string[]]$Arguments = @(),
    [ValidateRange(1, 20)][int]$Runs = 5,
    [ValidateRange(1, 600)][double]$TimeoutSeconds = 120,
    # The tree is settled when it uses less CPU than this share of one core
    # over each of $SettleSamples consecutive 250 ms samples.
    [ValidateRange(0.001, 1)][double]$SettleCpu = 0.02,
    [ValidateRange(1, 40)][int]$SettleSamples = 4,
    [string]$OutputFile
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class StartupWindows {
    delegate bool EnumProc(IntPtr hwnd, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int GetWindowTextLength(IntPtr hwnd);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    // Whether a visible top-level window with a title belongs to one of the processes.
    public static bool HasWindow(HashSet<uint> pids) {
        bool found = false;
        EnumWindows((hwnd, _) => {
            uint pid;
            GetWindowThreadProcessId(hwnd, out pid);
            if (pids.Contains(pid) && IsWindowVisible(hwnd) && GetWindowTextLength(hwnd) > 0) {
                found = true;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct ProcessEntry {
        public uint dwSize, cntUsage, th32ProcessID;
        public IntPtr th32DefaultHeapID;
        public uint th32ModuleID, cntThreads, th32ParentProcessID;
        public int pcPriClassBase;
        public uint dwFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string szExeFile;
    }
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint pid);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool Process32FirstW(IntPtr snapshot, ref ProcessEntry entry);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool Process32NextW(IntPtr snapshot, ref ProcessEntry entry);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    // The process and its descendants. A Toolhelp snapshot takes about a
    // millisecond, where a WMI query can take hundreds while the system is
    // busy launching, which would delay seeing the window.
    public static HashSet<uint> Tree(uint root) {
        var parents = new List<KeyValuePair<uint, uint>>();
        IntPtr snapshot = CreateToolhelp32Snapshot(2, 0);
        if (snapshot != new IntPtr(-1)) {
            var entry = new ProcessEntry { dwSize = (uint)Marshal.SizeOf(typeof(ProcessEntry)) };
            for (bool more = Process32FirstW(snapshot, ref entry); more; more = Process32NextW(snapshot, ref entry)) {
                parents.Add(new KeyValuePair<uint, uint>(entry.th32ProcessID, entry.th32ParentProcessID));
            }
            CloseHandle(snapshot);
        }
        var ids = new HashSet<uint> { root };
        bool added;
        do {
            added = false;
            foreach (var pair in parents) {
                if (ids.Contains(pair.Value) && ids.Add(pair.Key)) added = true;
            }
        } while (added);
        return ids;
    }
}
'@

function Get-Tree([int]$RootId) {
    , [StartupWindows]::Tree([uint32]$RootId)
}

function Get-TreeProcesses($ids) {
    @($ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
}

# Milliseconds of processor time used by the tree.
function Get-TreeCpu([int]$RootId) {
    $total = 0.0
    foreach ($process in (Get-TreeProcesses (Get-Tree $RootId))) {
        try { $total += $process.TotalProcessorTime.TotalMilliseconds } catch { }
    }
    $total
}

function Measure-Run([string]$Path) {
    $info = [Diagnostics.ProcessStartInfo]::new($Path)
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $info.WorkingDirectory = Split-Path -Parent $Path
    $info.UseShellExecute = $false
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $root = [Diagnostics.Process]::Start($info)
    try {
        $window = $null
        $ids = Get-Tree $root.Id
        $refreshed = $clock.Elapsed.TotalMilliseconds
        while ($clock.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
            if ($clock.Elapsed.TotalMilliseconds - $refreshed -ge 50) {
                $ids = Get-Tree $root.Id
                $refreshed = $clock.Elapsed.TotalMilliseconds
            }
            if ([StartupWindows]::HasWindow($ids)) { $window = $clock.Elapsed.TotalMilliseconds; break }
            if ($root.HasExited) { throw "$Path exited with $($root.ExitCode) before showing a window" }
            Start-Sleep -Milliseconds 5
        }
        if ($null -eq $window) { throw "$Path showed no window within $TimeoutSeconds s" }
        $quiet = 0
        $previous = Get-TreeCpu $root.Id
        $previousTime = $clock.Elapsed
        $settled = $null
        while ($clock.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
            Start-Sleep -Milliseconds 250
            $cpu = Get-TreeCpu $root.Id
            $now = $clock.Elapsed
            $share = ($cpu - $previous) / ($now - $previousTime).TotalMilliseconds
            $previous = $cpu
            $previousTime = $now
            if ($share -lt $SettleCpu) { $quiet++ } else { $quiet = 0 }
            if ($quiet -ge $SettleSamples) {
                $settled = ($now.TotalMilliseconds - 250 * $SettleSamples)
                break
            }
        }
        if ($null -eq $settled) { $settled = $clock.Elapsed.TotalMilliseconds }
        $processes = Get-TreeProcesses (Get-Tree $root.Id)
        [ordered]@{
            window_ms = [math]::Round($window, 1)
            settled_ms = [math]::Round($settled, 1)
            processes = $processes.Count
            working_set_mb = [math]::Round(($processes | Measure-Object -Property WorkingSet64 -Sum).Sum / 1MB, 1)
            private_mb = [math]::Round(($processes | Measure-Object -Property PrivateMemorySize64 -Sum).Sum / 1MB, 1)
        }
    } finally {
        foreach ($id in (Get-Tree $root.Id)) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 500
    }
}

function Get-Median([double[]]$Values) {
    $sorted = @($Values | Sort-Object)
    $middle = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 0) { ($sorted[$middle - 1] + $sorted[$middle]) / 2 } else { $sorted[$middle] }
}

$results = @()
foreach ($path in $Executable) {
    $path = (Resolve-Path -LiteralPath $path).Path
    # PowerShell names are case-insensitive: `$runs` would be the `$Runs` parameter.
    $measured = @()
    for ($run = 1; $run -le $Runs; $run++) { $measured += [pscustomobject](Measure-Run $path) }
    $results += [ordered]@{
        executable = $path
        size_mb = [math]::Round((Get-ChildItem -LiteralPath (Split-Path -Parent $path) -Recurse -File | Measure-Object -Property Length -Sum).Sum / 1MB, 1)
        runs = $measured
        median = [ordered]@{
            window_ms = Get-Median $measured.window_ms
            settled_ms = Get-Median $measured.settled_ms
            working_set_mb = Get-Median $measured.working_set_mb
            private_mb = Get-Median $measured.private_mb
        }
    }
}
$json = $results | ConvertTo-Json -Depth 6
if ($OutputFile) { $json | Set-Content -LiteralPath $OutputFile }
$json
