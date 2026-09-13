# 枚举 mini-todo（debug 实例）进程的可见顶层窗口：标题、类名、窗口矩形、客户区屏幕原点、owner、zIndex
[CmdletBinding()]
param([string]$ExePath = 'target\debug\mini-todo.exe', [string]$Title = '')
$ErrorActionPreference = 'Stop'
if (-not ('E2E.Win' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace E2E {
  public static class Win {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr hWnd, uint cmd);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr hWnd, int idx);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT p);
    [DllImport("user32.dll")] public static extern IntPtr GetShellWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    public static string Text(IntPtr h) { var sb = new StringBuilder(256); GetWindowText(h, sb, 256); return sb.ToString(); }
    public static string Class(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
    public static uint Pid(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
    public static List<IntPtr> TopLevel() { var l = new List<IntPtr>(); EnumWindows((h, _) => { l.Add(h); return true; }, IntPtr.Zero); return l; }
  }
}
'@
}
$W = [E2E.Win]
[void]$W::SetThreadDpiAwarenessContext([IntPtr]::new(-4))

function Get-ZIndex([IntPtr]$target) {
    $h = $W::GetWindow($W::GetShellWindow(), 1); $i = 0
    while ($h -ne [IntPtr]::Zero) {
        if ($W::IsWindowVisible($h)) { if ($h -eq $target) { return $i }; $i++ }
        $h = $W::GetWindow($h, 3)
    }
    return -1
}

$procs = Get-Process -Name 'mini-todo' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "*$ExePath*" }
if (-not $procs) { throw "no mini-todo process matching $ExePath" }
$pids = $procs | ForEach-Object { [uint32]$_.Id }
$fg = $W::GetForegroundWindow()
foreach ($h in $W::TopLevel()) {
    if (-not ($pids -contains $W::Pid($h))) { continue }
    if (-not $W::IsWindowVisible($h)) { continue }
    $t = $W::Text($h)
    if ($Title -and $t -ne $Title) { continue }
    $r = New-Object E2E.Win+RECT; [void]$W::GetWindowRect($h, [ref]$r)
    $c = New-Object E2E.Win+RECT; [void]$W::GetClientRect($h, [ref]$c)
    $p = New-Object E2E.Win+POINT; $p.X = 0; $p.Y = 0; [void]$W::ClientToScreen($h, [ref]$p)
    [pscustomobject]@{
        hwnd      = ('0x{0:X}' -f $h.ToInt64())
        title     = $t
        class     = $W::Class($h)
        rect      = "($($r.L),$($r.T))-($($r.R),$($r.B))"
        clientOrigin = "($($p.X),$($p.Y))"
        clientSize = "$($c.R)x$($c.B)"
        owner     = ('0x{0:X}' -f ([int64]$W::GetWindowLongPtr($h, -8)))
        zIndex    = (Get-ZIndex $h)
        iconic    = $W::IsIconic($h)
        dpi       = $W::GetDpiForWindow($h)
        isForeground = ($h -eq $fg)
    }
}
Write-Output ("foreground: 0x{0:X} [{1}] '{2}'" -f $fg.ToInt64(), $W::Class($fg), $W::Text($fg))
