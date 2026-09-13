# 桌面模式 e2e 检查脚本（只读查询 + Shell 动作，不改窗口样式；模式切换走 cdp.mjs 点 DOM 按钮）
# 用法：pwsh -NoProfile -File e2e.ps1 -Step <state|zorder|win-d|win-m|click|shot|restart-explorer|wait-window|kill-app>
[CmdletBinding()]
param(
    [ValidateSet('state', 'zorder', 'win-d', 'win-m', 'click', 'shot', 'restart-explorer', 'wait-window', 'kill-app', 'json')]
    [string]$Step = 'state',
    [string]$ExePath = 'target\debug\mini-todo.exe',
    [string]$Out = "$env:TEMP\mini-todo-e2e.png",
    [int]$X = 60,
    [int]$Y = 60,
    [int]$TimeoutSec = 120
)
$ErrorActionPreference = 'Stop'

if (-not ('E2E.Native' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace E2E {
  public static class Native {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr hWnd, uint cmd);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr hWnd, int idx);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern IntPtr GetShellWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr hWnd, uint flags);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    public const int GWL_STYLE = -16, GWL_EXSTYLE = -20, GWLP_HWNDPARENT = -8;
    public const uint GW_HWNDLAST = 1, GW_HWNDNEXT = 2, GW_HWNDPREV = 3, GW_OWNER = 4, GA_ROOT = 2;
    public const long WS_MINIMIZEBOX = 0x00020000L, WS_THICKFRAME = 0x00040000L;
    public const long WS_EX_TOOLWINDOW = 0x80L, WS_EX_APPWINDOW = 0x40000L, WS_EX_TOPMOST = 0x8L, WS_EX_NOACTIVATE = 0x08000000L;
    public static string Text(IntPtr h) { var sb = new StringBuilder(256); GetWindowText(h, sb, 256); return sb.ToString(); }
    public static string Class(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
    public static uint Pid(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
    public static List<IntPtr> TopLevel() { var l = new List<IntPtr>(); EnumWindows((h, _) => { l.Add(h); return true; }, IntPtr.Zero); return l; }
  }
}
'@
}
$N = [E2E.Native]
[void]$N::SetThreadDpiAwarenessContext([IntPtr]::new(-4))

function Find-MainWindow([switch]$Quiet) {
    $procs = Get-Process -Name 'mini-todo' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "*$ExePath*" }
    if (-not $procs) { if ($Quiet) { return [IntPtr]::Zero }; throw "没有找到路径匹配 '$ExePath' 的 mini-todo 进程" }
    $pids = $procs | ForEach-Object { [uint32]$_.Id }
    foreach ($h in $N::TopLevel()) { if ($pids -contains $N::Pid($h) -and $N::Text($h) -eq 'Mini Todo') { return $h } }
    if ($Quiet) { return [IntPtr]::Zero }
    throw '进程存在但没找到标题为 Mini Todo 的窗口'
}

function Get-ZIndex([IntPtr]$target) {
    # 从 Z 序最底（Progman）往上数可见顶层窗口，返回 target 的序号；-1 表示没找到
    $h = $N::GetWindow($N::GetShellWindow(), $N::GW_HWNDLAST)
    $i = 0
    while ($h -ne [IntPtr]::Zero) {
        if ($N::IsWindowVisible($h)) { if ($h -eq $target) { return $i }; $i++ }
        $h = $N::GetWindow($h, $N::GW_HWNDPREV)
    }
    return -1
}

function Get-State {
    $h = Find-MainWindow
    $r = New-Object E2E.Native+RECT; [void]$N::GetWindowRect($h, [ref]$r)
    $style = [int64]$N::GetWindowLongPtr($h, $N::GWL_STYLE)
    $ex = [int64]$N::GetWindowLongPtr($h, $N::GWL_EXSTYLE)
    $owner = [int64]$N::GetWindowLongPtr($h, $N::GWLP_HWNDPARENT)
    $progman = $N::GetShellWindow()
    [pscustomobject]@{
        hwnd          = ('0x{0:X}' -f $h.ToInt64())
        pid           = $N::Pid($h)
        rect          = "($($r.L),$($r.T))-($($r.R),$($r.B))"
        visible       = $N::IsWindowVisible($h)
        iconic        = $N::IsIconic($h)
        owner         = ('0x{0:X}' -f $owner)
        progman       = ('0x{0:X}' -f $progman.ToInt64())
        ownerIsProgman = ($owner -ne 0 -and $owner -eq $progman.ToInt64())
        zIndex        = (Get-ZIndex $h)
        minimizeBox   = [bool]($style -band $N::WS_MINIMIZEBOX)
        thickFrame    = [bool]($style -band $N::WS_THICKFRAME)
        toolWindow    = [bool]($ex -band $N::WS_EX_TOOLWINDOW)
        appWindow     = [bool]($ex -band $N::WS_EX_APPWINDOW)
        topmost       = [bool]($ex -band $N::WS_EX_TOPMOST)
        noActivate    = [bool]($ex -band $N::WS_EX_NOACTIVATE)
        dpi           = $N::GetDpiForWindow($h)
        foreground    = ('0x{0:X}' -f $N::GetForegroundWindow().ToInt64())
    }
}

function Show-ZOrder {
    $h = $N::GetWindow($N::GetShellWindow(), $N::GW_HWNDLAST)
    $i = 0
    while ($h -ne [IntPtr]::Zero -and $i -lt 12) {
        if ($N::IsWindowVisible($h)) {
            Write-Output ("z[{0,2}] 0x{1:X} [{2}] '{3}' pid={4}" -f $i, $h.ToInt64(), $N::Class($h), $N::Text($h), $N::Pid($h)); $i++
        }
        $h = $N::GetWindow($h, $N::GW_HWNDPREV)
    }
}

function Watch-ShellAction([scriptblock]$action, [int]$ms = 1500) {
    $h = Find-MainWindow
    & $action
    $everIconic = $false; $everHidden = $false; $rects = New-Object System.Collections.Generic.HashSet[string]
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $ms) {
        if ($N::IsIconic($h)) { $everIconic = $true }
        if (-not $N::IsWindowVisible($h)) { $everHidden = $true }
        $r = New-Object E2E.Native+RECT; [void]$N::GetWindowRect($h, [ref]$r)
        [void]$rects.Add("($($r.L),$($r.T))-($($r.R),$($r.B))")
        Start-Sleep -Milliseconds 20
    }
    # 窗口中心点命中测试：命中的顶层窗口是不是我们自己（被桌面盖住时会命中 Progman/WorkerW）
    $r = New-Object E2E.Native+RECT; [void]$N::GetWindowRect($h, [ref]$r)
    $p = New-Object E2E.Native+POINT; $p.X = [int](($r.L + $r.R) / 2); $p.Y = [int](($r.T + $r.B) / 2)
    $hit = $N::GetAncestor($N::WindowFromPoint($p), $N::GA_ROOT)
    [pscustomobject]@{
        everIconic = $everIconic; everHidden = $everHidden; rectsSeen = ($rects -join ' ; ')
        zIndexAfter = (Get-ZIndex $h)
        centerHitIsMain = ($hit -eq $h)
        centerHit = ('0x{0:X} [{1}]' -f $hit.ToInt64(), $N::Class($hit))
    }
}

function Save-Shot {
    Add-Type -AssemblyName System.Drawing
    $h = Find-MainWindow
    $r = New-Object E2E.Native+RECT; [void]$N::GetWindowRect($h, [ref]$r)
    $pad = 40
    $x = $r.L - $pad; $y = $r.T - $pad; $w = ($r.R - $r.L) + 2 * $pad; $hh = ($r.B - $r.T) + 2 * $pad
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
    $g.Dispose(); $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    Write-Output "saved $Out (screen rect $x,$y ${w}x$hh)"
}

function Invoke-Click {
    $h = Find-MainWindow
    $r = New-Object E2E.Native+RECT; [void]$N::GetWindowRect($h, [ref]$r)
    $sx = $r.L + $X; $sy = $r.T + $Y
    [void]$N::SetCursorPos($sx, $sy); Start-Sleep -Milliseconds 100
    $N::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero); $N::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 400
    Write-Output "clicked at screen ($sx,$sy)"
    Get-State | Format-List
}

switch ($Step) {
    'state'  { Get-State | Format-List }
    'json'   { Get-State | ConvertTo-Json -Compress }
    'zorder' { Show-ZOrder }
    'win-d'  { Watch-ShellAction { (New-Object -ComObject Shell.Application).ToggleDesktop() } | Format-List }
    'win-m'  { Watch-ShellAction { (New-Object -ComObject Shell.Application).MinimizeAll() } | Format-List }
    'click'  { Invoke-Click }
    'shot'   { Save-Shot }
    'wait-window' {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
            $h = Find-MainWindow -Quiet
            if ($h -ne [IntPtr]::Zero -and $N::IsWindowVisible($h)) { Write-Output ("window ready after {0:N0}s" -f $sw.Elapsed.TotalSeconds); exit 0 }
            Start-Sleep -Milliseconds 500
        }
        Write-Output "timeout waiting for window"; exit 1
    }
    'kill-app' {
        Get-Process -Name 'mini-todo' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "*$ExePath*" } | Stop-Process -Force
        Write-Output 'killed'
    }
    'restart-explorer' {
        $h = Find-MainWindow
        $before = Get-State
        Write-Output ("before: owner={0} progman={1} zIndex={2}" -f $before.owner, $before.progman, $before.zIndex)
        Stop-Process -Name explorer -Force
        Start-Sleep -Milliseconds 1500
        if (-not (Get-Process -Name explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }
        # 等 Progman 回来，再给轮询线程最多 5 秒重挂
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        while ($sw.Elapsed.TotalSeconds -lt 15 -and $N::GetShellWindow() -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 200 }
        Write-Output ("progman back after {0:N1}s: 0x{1:X}" -f $sw.Elapsed.TotalSeconds, $N::GetShellWindow().ToInt64())
        Write-Output ("main window alive: " + $N::IsWindow($h))
        $sw2 = [System.Diagnostics.Stopwatch]::StartNew()
        $ok = $false
        while ($sw2.Elapsed.TotalSeconds -lt 8) {
            $owner = [int64]$N::GetWindowLongPtr($h, $N::GWLP_HWNDPARENT)
            if ($N::IsWindow($h) -and $owner -ne 0 -and $owner -eq $N::GetShellWindow().ToInt64()) { $ok = $true; break }
            Start-Sleep -Milliseconds 200
        }
        Write-Output ("reattached within {0:N1}s: {1}" -f $sw2.Elapsed.TotalSeconds, $ok)
        Get-State | Format-List
    }
}
