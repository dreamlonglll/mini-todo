# 绝对屏幕坐标的鼠标 / 键盘辅助（物理像素，per-monitor DPI aware）
[CmdletBinding()]
param(
    [ValidateSet('click', 'dblclick', 'keys', 'wheel', 'drag', 'fg', 'hit')]
    [string]$Action = 'fg',
    [int]$X = 0, [int]$Y = 0,
    [int]$X2 = 0, [int]$Y2 = 0,
    [int]$Delta = -120,
    [string]$Text = '',
    [int]$SettleMs = 300
)
$ErrorActionPreference = 'Stop'
if (-not ('E2E.Mouse' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
namespace E2E {
  public static class Mouse {
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr hWnd, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    public static string Text(IntPtr h) { var sb = new StringBuilder(256); GetWindowText(h, sb, 256); return sb.ToString(); }
    public static string Class(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
    public static string Describe(IntPtr h) { return string.Format("0x{0:X} [{1}] '{2}'", h.ToInt64(), Class(h), Text(h)); }
  }
}
'@
}
$M = [E2E.Mouse]
[void]$M::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
$LDOWN = 0x2; $LUP = 0x4; $WHEEL = 0x800

function Describe-Point([int]$px, [int]$py) {
    $p = New-Object E2E.Mouse+POINT; $p.X = $px; $p.Y = $py
    $root = $M::GetAncestor($M::WindowFromPoint($p), 2)
    return $M::Describe($root)
}

switch ($Action) {
    'fg'  { Write-Output ("foreground: " + $M::Describe($M::GetForegroundWindow())) }
    'hit' { Write-Output ("hit($X,$Y): " + (Describe-Point $X $Y)) }
    'click' {
        [void]$M::SetCursorPos($X, $Y); Start-Sleep -Milliseconds 80
        $M::mouse_event($LDOWN, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40; $M::mouse_event($LUP, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds $SettleMs
        Write-Output ("clicked ($X,$Y) hit=" + (Describe-Point $X $Y) + " fg=" + $M::Describe($M::GetForegroundWindow()))
    }
    'dblclick' {
        [void]$M::SetCursorPos($X, $Y); Start-Sleep -Milliseconds 80
        for ($i = 0; $i -lt 2; $i++) { $M::mouse_event($LDOWN, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 30; $M::mouse_event($LUP, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 60 }
        Start-Sleep -Milliseconds $SettleMs
        Write-Output ("dblclicked ($X,$Y) fg=" + $M::Describe($M::GetForegroundWindow()))
    }
    'keys' {
        Add-Type -AssemblyName System.Windows.Forms
        [System.Windows.Forms.SendKeys]::SendWait($Text)
        Start-Sleep -Milliseconds $SettleMs
        Write-Output ("sent keys '$Text' fg=" + $M::Describe($M::GetForegroundWindow()))
    }
    'wheel' {
        [void]$M::SetCursorPos($X, $Y); Start-Sleep -Milliseconds 80
        $d = [int64]$Delta; if ($d -lt 0) { $d += 4294967296 }
        $M::mouse_event($WHEEL, 0, 0, [uint32]$d, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds $SettleMs
        Write-Output ("wheel $Delta at ($X,$Y)")
    }
    'drag' {
        [void]$M::SetCursorPos($X, $Y); Start-Sleep -Milliseconds 80
        $M::mouse_event($LDOWN, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 150
        $steps = 20
        for ($i = 1; $i -le $steps; $i++) {
            $cx = [int]($X + ($X2 - $X) * $i / $steps); $cy = [int]($Y + ($Y2 - $Y) * $i / $steps)
            [void]$M::SetCursorPos($cx, $cy); Start-Sleep -Milliseconds 25
        }
        Start-Sleep -Milliseconds 150
        $M::mouse_event($LUP, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds $SettleMs
        Write-Output ("dragged ($X,$Y) -> ($X2,$Y2)")
    }
}
