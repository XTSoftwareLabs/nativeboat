# Typing comparison for desktop editors, such as a native NativeBoat build
# and the original Electron package of the same application. For each run:
# launch with a document and an isolated profile, wait until the process
# tree's CPU use settles, click into the text, type characters at a steady
# pace, then record the processor time the tree used from the first key
# until it settled again and how long after the last key that took.
# Reports medians per executable as JSON.
#
# Input is posted to the application's own window: the mouse, the keyboard
# focus and whatever window the user works in are left alone.
param(
    [Parameter(Mandatory)][string[]]$Executable,
    [Parameter(Mandatory)][string]$Document,
    [ValidateRange(1, 20)][int]$Runs = 3,
    [string]$Text = 'the quick brown fox jumps over the lazy dog',
    [ValidateRange(0, 1000)][int]$KeyIntervalMs = 30,
    # Where to click, as fractions of the window's content.
    [double]$ClickX = 0.235,
    [double]$ClickY = 0.091,
    [ValidateRange(0.001, 1)][double]$SettleCpu = 0.02,
    # Saves each run's window after typing, to check the text arrived.
    [string]$ScreenshotDirectory,
    [string]$OutputFile
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @'
using System; using System.Collections.Generic; using System.Runtime.InteropServices; using System.Text;
public static class PostedInput {
    delegate bool EnumProc(IntPtr hwnd, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc proc, IntPtr data);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern short VkKeyScanW(char c);
    [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint code, uint type);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    public struct RECT { public int Left, Top, Right, Bottom; }
    // The window that takes the page's input: Chromium's render widget
    // where there is one, else the top-level window.
    public static IntPtr Target(IntPtr top) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(top, (h, d) => {
            var name = new StringBuilder(64); GetClassName(h, name, 64);
            if (name.ToString() == "Chrome_RenderWidgetHostHWND") { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found == IntPtr.Zero ? top : found;
    }
    public static void Click(IntPtr target, int x, int y) {
        IntPtr point = (IntPtr)((y << 16) | (x & 0xFFFF));
        PostMessage(target, 0x0200, IntPtr.Zero, point);      // WM_MOUSEMOVE
        PostMessage(target, 0x0201, (IntPtr)1, point);        // WM_LBUTTONDOWN
        PostMessage(target, 0x0202, IntPtr.Zero, point);      // WM_LBUTTONUP
    }
    public static void Press(IntPtr target, char c) {
        uint vk = (uint)(VkKeyScanW(c) & 0xFF);
        uint scan = MapVirtualKeyW(vk, 0);
        // The application's message loop translates the key into its
        // character, as for typed keys.
        PostMessage(target, 0x0100, (IntPtr)vk, (IntPtr)(1 | (scan << 16)));                       // WM_KEYDOWN
        PostMessage(target, 0x0101, (IntPtr)vk, (IntPtr)(1 | (scan << 16) | (3u << 30)));          // WM_KEYUP
    }
}
'@
# Physical pixels for client coordinates.
[void][PostedInput]::SetProcessDpiAwarenessContext([IntPtr]-4)

function Get-Tree([int]$RootId) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId
    $ids = [System.Collections.Generic.List[int]]::new()
    $ids.Add($RootId)
    do {
        $added = $false
        foreach ($process in $all) {
            if ($ids.Contains([int]$process.ParentProcessId) -and -not $ids.Contains([int]$process.ProcessId)) {
                $ids.Add([int]$process.ProcessId); $added = $true
            }
        }
    } while ($added)
    , $ids
}

function Get-TreeCpu($ids) {
    $total = 0.0
    foreach ($id in $ids) {
        try { $total += (Get-Process -Id $id).TotalProcessorTime.TotalMilliseconds } catch { }
    }
    $total
}

# Waits until the tree uses less than the settle share of a core over four
# consecutive 250 ms samples; returns when it last was busy.
function Wait-Settled($ids, $clock) {
    $quiet = 0
    $previous = Get-TreeCpu $ids
    $previousTime = $clock.Elapsed
    while ($clock.Elapsed.TotalSeconds -lt 60) {
        Start-Sleep -Milliseconds 250
        $cpu = Get-TreeCpu $ids
        $now = $clock.Elapsed
        $share = ($cpu - $previous) / [Math]::Max(1, ($now - $previousTime).TotalMilliseconds)
        $previous = $cpu
        $previousTime = $now
        if ($share -lt $SettleCpu) { $quiet++ } else { $quiet = 0 }
        if ($quiet -ge 4) { return $now.TotalMilliseconds - 1000 }
    }
    $clock.Elapsed.TotalMilliseconds
}

function Measure-Typing([string]$Path) {
    $profileDir = Join-Path ([IO.Path]::GetTempPath()) ('nativeboat-typing-' + [guid]::NewGuid().ToString('n'))
    New-Item -ItemType Directory -Force "$profileDir\Roaming", "$profileDir\Local" | Out-Null
    $info = [Diagnostics.ProcessStartInfo]::new($Path)
    $info.ArgumentList.Add($Document)
    # Electron reads its profile location from the command line.
    $info.ArgumentList.Add("--user-data-dir=$profileDir\electron")
    $info.Environment['APPDATA'] = "$profileDir\Roaming"
    $info.Environment['LOCALAPPDATA'] = "$profileDir\Local"
    $info.UseShellExecute = $false
    $root = [Diagnostics.Process]::Start($info)
    try {
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $window = [IntPtr]::Zero
        while ($clock.Elapsed.TotalSeconds -lt 60 -and $window -eq [IntPtr]::Zero) {
            Start-Sleep -Milliseconds 100
            foreach ($id in (Get-Tree $root.Id)) {
                try { $handle = (Get-Process -Id $id).MainWindowHandle } catch { continue }
                if ($handle -ne [IntPtr]::Zero) { $window = $handle; break }
            }
        }
        if ($window -eq [IntPtr]::Zero) { throw "$Path showed no window" }
        $ids = Get-Tree $root.Id
        [void](Wait-Settled $ids $clock)
        $ids = Get-Tree $root.Id
        $target = [PostedInput]::Target($window)
        $rect = New-Object PostedInput+RECT
        [void][PostedInput]::GetClientRect($target, [ref]$rect)
        [PostedInput]::Click($target, [int]($rect.Right * $ClickX), [int]($rect.Bottom * $ClickY))
        [void](Wait-Settled $ids $clock)
        $before = Get-TreeCpu $ids
        $start = $clock.Elapsed.TotalMilliseconds
        foreach ($char in $Text.ToCharArray()) {
            [PostedInput]::Press($target, $char)
            Start-Sleep -Milliseconds $KeyIntervalMs
        }
        $typed = $clock.Elapsed.TotalMilliseconds
        $settled = Wait-Settled $ids $clock
        $cpu = (Get-TreeCpu $ids) - $before
        if ($ScreenshotDirectory) {
            New-Item -ItemType Directory -Force $ScreenshotDirectory | Out-Null
            $bounds = New-Object PostedInput+RECT
            [void][PostedInput]::GetWindowRect($window, [ref]$bounds)
            $bitmap = New-Object System.Drawing.Bitmap ([Math]::Max(1, $bounds.Right - $bounds.Left)), ([Math]::Max(1, $bounds.Bottom - $bounds.Top))
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            $dc = $graphics.GetHdc(); [void][PostedInput]::PrintWindow($window, $dc, 2); $graphics.ReleaseHdc($dc)
            $name = [IO.Path]::GetFileNameWithoutExtension($Path) + '-' + [guid]::NewGuid().ToString('n').Substring(0, 6) + '.png'
            $bitmap.Save((Join-Path $ScreenshotDirectory $name))
        }
        [ordered]@{
            cpu_ms = [math]::Round($cpu, 1)
            cpu_per_key_ms = [math]::Round($cpu / $Text.Length, 2)
            typing_ms = [math]::Round($typed - $start, 1)
            settled_after_last_key_ms = [math]::Round([Math]::Max(0, $settled - $typed), 1)
        }
    } finally {
        foreach ($id in (Get-Tree $root.Id)) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 500
        Remove-Item -Recurse -Force $profileDir -ErrorAction SilentlyContinue
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
    $measured = @()
    for ($run = 1; $run -le $Runs; $run++) { $measured += [pscustomobject](Measure-Typing $path) }
    $results += [ordered]@{
        executable = $path
        characters = $Text.Length
        runs = $measured
        median = [ordered]@{
            cpu_ms = Get-Median $measured.cpu_ms
            cpu_per_key_ms = Get-Median $measured.cpu_per_key_ms
            settled_after_last_key_ms = Get-Median $measured.settled_after_last_key_ms
        }
    }
}
$json = $results | ConvertTo-Json -Depth 6
if ($OutputFile) { $json | Set-Content -LiteralPath $OutputFile }
$json
