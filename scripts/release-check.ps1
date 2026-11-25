# Builds nativeboat in release mode, compiles the React + Vite example with the
# engine-free `nativeboat build` and checks the result against size, startup and
# memory budgets. With -Electron it also measures the same application running
# under Electron for comparison (the example's dev dependencies and `vite build`
# output must be present).
#
#   ./scripts/release-check.ps1            # build, measure, enforce budgets
#   ./scripts/release-check.ps1 -NoBuild   # reuse target/release
#   ./scripts/release-check.ps1 -Electron  # also measure Electron
#   ./scripts/release-check.ps1 -BuildProfile debug -NoBuild   # quick try
param(
    [string]$Example = (Join-Path $PSScriptRoot '..\examples\react-vite'),
    [int]$Runs = 5,
    [double]$MaxSizeMB = 16,
    [double]$MaxStartupMs = 1500,
    [double]$MaxMemoryMB = 80,
    [ValidateSet('release', 'debug')][string]$BuildProfile = 'release',
    [switch]$NoBuild,
    [switch]$Electron
)

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$Example = Resolve-Path $Example
$release = Join-Path $root "target\$BuildProfile"

Add-Type @"
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public sealed class NativeBoatRun {
    public int Pid;
    public IntPtr Window;
    public double WindowMs = -1;
    public double ReadyMs = -1;
    public Process Process;
}

public static class NativeBoatImports {
    static uint U32(byte[] d, long o) { return BitConverter.ToUInt32(d, (int)o); }
    static long Offset(byte[] d, int sections, int count, uint rva) {
        for (int i = 0; i < count; i++) {
            long s = sections + 40 * i;
            uint size = Math.Max(U32(d, s + 8), U32(d, s + 16)), va = U32(d, s + 12);
            if (rva >= va && rva < va + size) return rva - va + U32(d, s + 20);
        }
        return -1;
    }
    // The DLLs a PE file imports, directly or delay-loaded.
    public static List<string> Of(string path) {
        var d = System.IO.File.ReadAllBytes(path);
        int pe = (int)U32(d, 0x3C);
        int count = BitConverter.ToUInt16(d, pe + 6);
        int optional = pe + 24;
        int directories = optional + (BitConverter.ToUInt16(d, optional) == 0x20B ? 112 : 96);
        int sections = optional + BitConverter.ToUInt16(d, pe + 20);
        var names = new List<string>();
        foreach (var table in new[] { new[] { 1, 20, 12 }, new[] { 13, 32, 4 } }) {
            uint rva = U32(d, directories + 8 * table[0]);
            if (rva == 0) continue;
            for (long entry = Offset(d, sections, count, rva); entry >= 0; entry += table[1]) {
                uint name = U32(d, entry + table[2]);
                if (name == 0) break;
                long at = Offset(d, sections, count, name);
                int end = Array.IndexOf(d, (byte)0, (int)at);
                names.Add(Encoding.ASCII.GetString(d, (int)at, end - (int)at));
            }
        }
        return names;
    }
}

public static class NativeBoatProbe {
    delegate bool EnumProc(IntPtr hwnd, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc proc, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);

    static string Text(IntPtr hwnd) { var b = new StringBuilder(256); GetWindowText(hwnd, b, 256); return b.ToString(); }
    static string Class(IntPtr hwnd) { var b = new StringBuilder(64); GetClassName(hwnd, b, 64); return b.ToString(); }

    // The visible top-level window of the process with the given title.
    static IntPtr Window(uint pid, string title) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, d) => {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner == pid && IsWindowVisible(h) && Text(h) == title) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    // Whether the window contains a native button with the given caption.
    static bool HasButton(IntPtr window, string caption) {
        bool found = false;
        EnumChildWindows(window, (h, d) => {
            if (string.Equals(Class(h), "Button", StringComparison.OrdinalIgnoreCase) && Text(h) == caption) { found = true; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    // Starts a program and times its first window and, when a caption is
    // given, the native button that shows the user interface is ready.
    public static NativeBoatRun Start(string exe, string args, string appData, string title, string button, int timeoutMs) {
        var info = new ProcessStartInfo(exe, args);
        info.UseShellExecute = false;
        info.EnvironmentVariables["APPDATA"] = appData;
        info.EnvironmentVariables.Remove("NATIVEBOAT_HIDDEN");
        var run = new NativeBoatRun();
        var watch = Stopwatch.StartNew();
        run.Process = Process.Start(info);
        run.Pid = run.Process.Id;
        while (watch.ElapsedMilliseconds < timeoutMs && !run.Process.HasExited) {
            if (run.Window == IntPtr.Zero) {
                run.Window = Window((uint)run.Pid, title);
                if (run.Window != IntPtr.Zero) run.WindowMs = watch.Elapsed.TotalMilliseconds;
            }
            if (run.Window != IntPtr.Zero) {
                if (button == null) break;
                if (HasButton(run.Window, button)) { run.ReadyMs = watch.Elapsed.TotalMilliseconds; break; }
            }
            Thread.Sleep(1);
        }
        return run;
    }
}
"@

function Median([double[]]$values) {
    $sorted = $values | Sort-Object
    $sorted[[int][math]::Floor(($sorted.Count - 1) / 2)]
}

# The process and all its descendants. Get-Process is used rather than WMI,
# whose queries can stall on CI machines.
function Get-Tree([int]$rootId) {
    $all = @(Get-Process | ForEach-Object {
            $parent = try { $_.Parent.Id } catch { $null }
            [pscustomobject]@{
                ProcessId        = $_.Id
                ParentProcessId  = $parent
                WorkingSetSize   = $_.WorkingSet64
                PrivatePageCount = $_.PrivateMemorySize64
            }
        })
    $tree = @($all | Where-Object ProcessId -EQ $rootId)
    $queue = [System.Collections.Generic.Queue[int]]::new()
    $queue.Enqueue($rootId)
    while ($queue.Count -gt 0) {
        $parent = $queue.Dequeue()
        foreach ($child in $all | Where-Object ParentProcessId -EQ $parent) {
            $tree += $child
            $queue.Enqueue([int]$child.ProcessId)
        }
    }
    $tree
}

function Stop-Tree([int]$rootId) {
    foreach ($process in Get-Tree $rootId) {
        Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

# Launches a program $Runs times and measures startup, memory and processes.
function Measure-App([string]$exe, [string]$arguments, [string]$button) {
    $results = @()
    for ($i = 1; $i -le $Runs; $i++) {
        $run = [NativeBoatProbe]::Start($exe, $arguments, $appData, 'Notes', $button, 30000)
        if ($run.WindowMs -lt 0 -or ($button -and $run.ReadyMs -lt 0)) {
            Stop-Tree $run.Pid
            throw "$exe did not show its window within 30 s"
        }
        Start-Sleep -Seconds 3
        $tree = @(Get-Tree $run.Pid)
        $modules = @($run.Process.Modules | ForEach-Object ModuleName)
        [NativeBoatProbe]::PostMessage($run.Window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        $exited = $run.Process.WaitForExit(10000)
        $exitCode = if ($exited) { $run.Process.ExitCode } else { $null }
        Stop-Tree $run.Pid
        $results += [pscustomobject]@{
            WindowMs  = $run.WindowMs
            ReadyMs   = $run.ReadyMs
            MemoryMB  = ($tree | Measure-Object WorkingSetSize -Sum).Sum / 1MB
            PrivateMB = ($tree | Measure-Object PrivatePageCount -Sum).Sum / 1MB
            Processes = $tree.Count
            Modules   = $modules
            ExitCode  = $exitCode
        }
        Write-Host ("  run {0}: window {1:N0} ms, ready {2:N0} ms, {3:N1} MB in {4} process(es)" -f `
                $i, $run.WindowMs, $run.ReadyMs, $results[-1].MemoryMB, $tree.Count)
    }
    $results
}

if (-not $NoBuild) {
    $flags = @(if ($BuildProfile -eq 'release') { '--release' })
    # Linked by the toolchain's own linker against import libraries the build
    # makes itself: no Visual C++ linker or Windows SDK library is involved.
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = 'rust-lld'
    cargo build @flags --locked -p nativeboat-cli
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    # Builds use the toolchain shipped next to nativeboat.exe, as users' do.
    & (Join-Path $PSScriptRoot 'package-toolchain.ps1') -Destination (Join-Path $release 'toolchain')
}

# nativeboat, its toolchain and what it builds use no C runtime: they import
# operating-system DLLs only.
$runtime = '(?i)^(vcruntime|msvcp|msvcr|ucrtbase|concrt|vccorlib|api-ms-win-crt-)'
function Test-Imports([string]$Path) {
    $found = @([NativeBoatImports]::Of($Path) | Where-Object { $_ -match $runtime } | Sort-Object -Unique)
    if ($found) { "$(Split-Path -Leaf $Path) imports C runtime DLLs: $($found -join ', ')" }
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("nativeboat-release-check-" + [guid]::NewGuid().ToString('n'))
$out = Join-Path $work 'dist'
$appData = Join-Path $work 'appdata'
New-Item -ItemType Directory -Force $appData | Out-Null
$failures = @()
foreach ($binary in @(Join-Path $release 'nativeboat.exe') + @(Get-ChildItem (Join-Path $release 'toolchain') -Recurse -Include *.exe, *.dll -ErrorAction SilentlyContinue | ForEach-Object FullName)) {
    $failures += @(Test-Imports $binary)
}

try {
    Write-Host "Compiling $Example"
    & (Join-Path $release 'nativeboat.exe') build $Example --out $out
    if ($LASTEXITCODE) { throw "nativeboat build failed with exit code $LASTEXITCODE" }
    $outputs = @(Get-ChildItem $out)
    if ($outputs.Count -ne 1 -or $outputs[0].Name -ne 'Notes.exe') {
        $failures += "the output is not a single Notes.exe: $($outputs.Name -join ', ')"
    }
    $exe = Join-Path $out 'Notes.exe'
    $sizeMB = (Get-Item $exe).Length / 1MB
    $failures += @(Test-Imports $exe)

    Write-Host "Measuring Notes.exe"
    $native = Measure-App $exe '' 'Save'
    $startup = Median ($native | ForEach-Object ReadyMs)
    $window = Median ($native | ForEach-Object WindowMs)
    $memory = Median ($native | ForEach-Object MemoryMB)
    $private = Median ($native | ForEach-Object PrivateMB)

    $browser = '(?i)^(webview2loader|embeddedbrowserwebview|libcef|chrome_elf|chrome|electron|mshtml|edgehtml|ieframe|jscript9|chakra)\.dll$'
    $loaded = @($native | ForEach-Object { $_.Modules } | Where-Object { $_ -match $browser } | Sort-Object -Unique)
    if ($sizeMB -gt $MaxSizeMB) { $failures += ('size {0:N1} MB exceeds {1} MB' -f $sizeMB, $MaxSizeMB) }
    if ($startup -gt $MaxStartupMs) { $failures += ('startup {0:N0} ms exceeds {1} ms' -f $startup, $MaxStartupMs) }
    if ($memory -gt $MaxMemoryMB) { $failures += ('memory {0:N1} MB exceeds {1} MB' -f $memory, $MaxMemoryMB) }
    if ($native | Where-Object Processes -NE 1) { $failures += 'the application started additional processes' }
    if ($native | Where-Object ExitCode -NE 0) { $failures += 'closing the window did not exit with code 0' }
    if ($loaded) { $failures += "browser components were loaded: $($loaded -join ', ')" }

    $rows = @(
        ('| | nativeboat |' + $(if ($Electron) { ' Electron |' } else { '' })),
        ('| --- | --- |' + $(if ($Electron) { ' --- |' } else { '' }))
    )
    $electronRows = @{}
    if ($Electron) {
        $electronExe = Join-Path $Example 'node_modules\electron\dist\electron.exe'
        if (-not (Test-Path $electronExe)) { throw "Electron is not installed in $Example" }
        $distribution = (Get-ChildItem (Split-Path $electronExe) -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
        Write-Host "Measuring Electron"
        # Electron is only measured for comparison; its problems are not ours.
        try {
            $reference = Measure-App $electronExe "`"$Example`"" $null
            $electronRows = @{
                Size    = ' {0:N1} MB |' -f $distribution
                Window  = ' {0:N0} ms |' -f (Median ($reference | ForEach-Object WindowMs))
                Ready   = ' n/a |'
                Memory  = ' {0:N1} MB |' -f (Median ($reference | ForEach-Object MemoryMB))
                Private = ' {0:N1} MB |' -f (Median ($reference | ForEach-Object PrivateMB))
                Process = ' {0} |' -f (Median ($reference | ForEach-Object Processes))
            }
        } catch {
            Write-Host "warning: Electron could not be measured: $_"
            $electronRows = @{ Size = ' n/a |'; Window = ' n/a |'; Ready = ' n/a |'; Memory = ' n/a |'; Private = ' n/a |'; Process = ' n/a |' }
        }
    }
    $rows += ('| Distribution size | {0:N1} MB |' -f $sizeMB) + $electronRows.Size
    $rows += ('| Time to window (median of {0}) | {1:N0} ms |' -f $Runs, $window) + $electronRows.Window
    $rows += ('| Time to native UI ready | {0:N0} ms |' -f $startup) + $electronRows.Ready
    $rows += ('| Idle working set | {0:N1} MB |' -f $memory) + $electronRows.Memory
    $rows += ('| Idle private memory | {0:N1} MB |' -f $private) + $electronRows.Private
    $rows += ('| Processes | {0} |' -f (Median ($native | ForEach-Object Processes))) + $electronRows.Process
    $budget = 'Budgets: size {0} MB, startup {1} ms, memory {2} MB.' -f $MaxSizeMB, $MaxStartupMs, $MaxMemoryMB
    $report = ($rows + '', $budget) -join "`n"
    Write-Host ''
    Write-Host $report
    if ($env:GITHUB_STEP_SUMMARY) {
        "## Release check`n`n$report`n" | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY
    }
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($failures) {
    foreach ($failure in $failures) { Write-Host "error: $failure" }
    exit 1
}
Write-Host 'Release check passed.'
