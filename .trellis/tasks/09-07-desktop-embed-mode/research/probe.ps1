# 桌面模式原型探针脚本
#
# 用途：不改 Rust 代码，直接从 PowerShell 用 Win32 API 对 mini-todo 主窗口做实验，
# 验证「挂到桌面宿主 / owner=Progman」两种方案在 Win+D、透明、Z 序上的真实表现。
#
# 用法（PowerShell 7）：
#   .\probe.ps1 -Action state                      # 打印主窗口状态（父/owner/样式/矩形/是否最小化）
#   .\probe.ps1 -Action desktop-tree               # 打印 Progman / WorkerW / SHELLDLL_DefView 层级
#   .\probe.ps1 -Action toggle-desktop             # 等价 Win+D（Shell.ToggleDesktop）
#   .\probe.ps1 -Action shot -Out C:\x.png         # 截取主窗口所在屏幕区域（含周边 40px）
#   .\probe.ps1 -Action embed-child                # 方案 A：SetParent 到图标宿主，排到图标之上
#   .\probe.ps1 -Action embed-owner                # 方案 B：owner=Progman + 去 WS_MINIMIZEBOX + HWND_BOTTOM
#   .\probe.ps1 -Action restore                    # 还原为普通顶层窗口
#   .\probe.ps1 -Action click -X 100 -Y 100        # 在窗口客户区相对坐标模拟一次左键点击
#
# -ExePath 用来区分 dev 实例与安装版实例（默认匹配 target\debug）。

[CmdletBinding()]
param(
    [ValidateSet('state', 'desktop-tree', 'toggle-desktop', 'watch-toggle', 'shot', 'embed-child', 'embed-owner', 'restore', 'click', 'hover', 'move', 'zorder', 'minimize-all', 'undo-minimize', 'noactivate-on', 'noactivate-off')]
    [string]$Action = 'state',
    [string]$ExePath = 'target\debug\mini-todo.exe',
    [string]$Out = "$env:TEMP\mini-todo-probe.png",
    [int]$X = 40,
    [int]$Y = 40
)

$ErrorActionPreference = 'Stop'

if (-not ('Probe.Native' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace Probe {
  public static class Native {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr hWndParent, EnumWindowsProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr hWnd, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr hWnd, uint cmd);
    [DllImport("user32.dll")] public static extern IntPtr SetParent(IntPtr hWndChild, IntPtr hWndNewParent);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr hWnd, int idx);
    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")] public static extern IntPtr SetWindowLongPtr(IntPtr hWnd, int idx, IntPtr val);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int cmd);
    // title 用 IntPtr 而不是 string：PowerShell 会把 $null 变成 ""，导致按空标题匹配失败
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, IntPtr title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, IntPtr title);
    [DllImport("user32.dll")] public static extern IntPtr GetShellWindow();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageTimeoutW(IntPtr hWnd, uint msg, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr hWnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hWnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int idx);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowDpiAwarenessContext(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetAwarenessFromDpiAwarenessContext(IntPtr ctx);

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

    public const int GWL_STYLE = -16, GWL_EXSTYLE = -20, GWLP_HWNDPARENT = -8;
    public const uint GA_PARENT = 1, GA_ROOT = 2, GA_ROOTOWNER = 3;
    public const uint GW_HWNDFIRST = 0, GW_HWNDLAST = 1, GW_HWNDNEXT = 2, GW_HWNDPREV = 3, GW_OWNER = 4, GW_CHILD = 5;
    public const uint SWP_NOSIZE = 0x1, SWP_NOMOVE = 0x2, SWP_NOZORDER = 0x4, SWP_NOACTIVATE = 0x10, SWP_FRAMECHANGED = 0x20, SWP_SHOWWINDOW = 0x40;
    public const long WS_CHILD = 0x40000000L, WS_POPUP = 0x80000000L, WS_MINIMIZEBOX = 0x00020000L, WS_MAXIMIZEBOX = 0x00010000L, WS_SYSMENU = 0x00080000L, WS_THICKFRAME = 0x00040000L, WS_VISIBLE = 0x10000000L;
    public const long WS_EX_TOOLWINDOW = 0x80L, WS_EX_APPWINDOW = 0x40000L, WS_EX_LAYERED = 0x80000L, WS_EX_TOPMOST = 0x8L, WS_EX_NOACTIVATE = 0x08000000L, WS_EX_TRANSPARENT = 0x20L, WS_EX_NOREDIRECTIONBITMAP = 0x00200000L;
    public static readonly IntPtr HWND_TOP = IntPtr.Zero, HWND_BOTTOM = new IntPtr(1), HWND_TOPMOST = new IntPtr(-1), HWND_NOTOPMOST = new IntPtr(-2);

    public static string Text(IntPtr h) { var sb = new StringBuilder(256); GetWindowText(h, sb, 256); return sb.ToString(); }
    public static string Class(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
    public static uint Pid(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }

    public static List<IntPtr> TopLevel() { var l = new List<IntPtr>(); EnumWindows((h, _) => { l.Add(h); return true; }, IntPtr.Zero); return l; }
    public static List<IntPtr> Children(IntPtr p) { var l = new List<IntPtr>(); EnumChildWindows(p, (h, _) => { l.Add(h); return true; }, IntPtr.Zero); return l; }
  }
}
'@
}

$N = [Probe.Native]
# pwsh 不是 Per-Monitor DPI aware，在 125% 副屏上 GetWindowRect / SetCursorPos / 截屏会被系统按 DPI 虚拟化，
# 顶层窗口与桌面子窗口的虚拟化结果还不一致，导致点击落点与截图坐标对不上。把当前线程切成 PMv2 后全部使用物理像素。
[void]$N::SetThreadDpiAwarenessContext([IntPtr]::new(-4))

function Describe([IntPtr]$h) {
    if ($h -eq [IntPtr]::Zero) { return '<null>' }
    $r = New-Object Probe.Native+RECT
    [void]$N::GetWindowRect($h, [ref]$r)
    $style = [int64]$N::GetWindowLongPtr($h, $N::GWL_STYLE)
    $ex = [int64]$N::GetWindowLongPtr($h, $N::GWL_EXSTYLE)
    $vis = if ($N::IsWindowVisible($h)) { 'V' } else { '-' }
    "0x{0:X} [{1}] '{2}' pid={3} {4} rect=({5},{6})-({7},{8}) style=0x{9:X8} ex=0x{10:X8}" -f $h.ToInt64(), $N::Class($h), $N::Text($h), $N::Pid($h), $vis, $r.L, $r.T, $r.R, $r.B, $style, $ex
}

function Find-MainWindow {
    $procs = Get-Process -Name 'mini-todo' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "*$ExePath*" }
    if (-not $procs) { throw "没有找到路径匹配 '$ExePath' 的 mini-todo 进程" }
    $pids = $procs | ForEach-Object { [uint32]$_.Id }
    # 主窗口标题固定为 "Mini Todo"（tauri.conf.json）；先在顶层找，找不到再在桌面宿主的子窗口里找（已嵌入的情况）
    $all = $N::TopLevel()
    foreach ($h in $all) { if ($pids -contains $N::Pid($h) -and $N::Text($h) -eq 'Mini Todo') { return $h } }
    # 注意：$host 是 PowerShell 只读自动变量，不能用作循环变量
    $hosts = Get-DesktopHosts
    foreach ($hostWnd in @($hosts.Progman) + $hosts.WorkerWs) {
        if ($hostWnd -eq [IntPtr]::Zero) { continue }
        foreach ($h in $N::Children($hostWnd)) { if ($pids -contains $N::Pid($h) -and $N::Text($h) -eq 'Mini Todo') { return $h } }
    }
    throw '进程存在但没找到标题为 Mini Todo 的窗口'
}

function Get-DesktopHosts {
    $progman = $N::GetShellWindow()
    $workerWs = @()
    foreach ($h in $N::TopLevel()) { if ($N::Class($h) -eq 'WorkerW') { $workerWs += $h } }
    # 24H2 之后 WorkerW 可能是 Progman 的子窗口
    foreach ($h in $N::Children($progman)) { if ($N::Class($h) -eq 'WorkerW') { $workerWs += $h } }
    # SHELLDLL_DefView 的宿主：先查 Progman 直属，再查各 WorkerW
    $defView = $N::FindWindowExW($progman, [IntPtr]::Zero, 'SHELLDLL_DefView', [IntPtr]::Zero)
    $iconHost = if ($defView -ne [IntPtr]::Zero) { $progman } else { [IntPtr]::Zero }
    if ($defView -eq [IntPtr]::Zero) {
        foreach ($w in $workerWs) {
            $d = $N::FindWindowExW($w, [IntPtr]::Zero, 'SHELLDLL_DefView', [IntPtr]::Zero)
            if ($d -ne [IntPtr]::Zero) { $defView = $d; $iconHost = $w; break }
        }
    }
    [pscustomobject]@{ Progman = $progman; WorkerWs = $workerWs; DefView = $defView; IconHost = $iconHost }
}

function Show-State {
    $h = Find-MainWindow
    $hosts = Get-DesktopHosts
    Write-Output ("main       : " + (Describe $h))
    Write-Output ("IsIconic   : " + $N::IsIconic($h))
    Write-Output ("GetParent  : " + (Describe $N::GetParent($h)))
    Write-Output ("GA_PARENT  : " + (Describe $N::GetAncestor($h, $N::GA_PARENT)))
    Write-Output ("GW_OWNER   : " + (Describe $N::GetWindow($h, $N::GW_OWNER)))
    Write-Output ("GA_ROOT    : " + (Describe $N::GetAncestor($h, $N::GA_ROOT)))
    Write-Output ("HWNDPARENT : 0x{0:X}" -f ([int64]$N::GetWindowLongPtr($h, $N::GWLP_HWNDPARENT)))
    Write-Output ("Progman    : " + (Describe $hosts.Progman))
    Write-Output ("IconHost   : " + (Describe $hosts.IconHost))
    Write-Output ("DefView    : " + (Describe $hosts.DefView))
    Write-Output ("VirtualScr : x={0} y={1} w={2} h={3}" -f $N::GetSystemMetrics(76), $N::GetSystemMetrics(77), $N::GetSystemMetrics(78), $N::GetSystemMetrics(79))
    $style = [int64]$N::GetWindowLongPtr($h, $N::GWL_STYLE)
    $ex = [int64]$N::GetWindowLongPtr($h, $N::GWL_EXSTYLE)
    $flags = @()
    if ($style -band $N::WS_CHILD) { $flags += 'WS_CHILD' }
    if ($style -band $N::WS_POPUP) { $flags += 'WS_POPUP' }
    if ($style -band $N::WS_MINIMIZEBOX) { $flags += 'WS_MINIMIZEBOX' }
    if ($style -band $N::WS_SYSMENU) { $flags += 'WS_SYSMENU' }
    if ($style -band $N::WS_THICKFRAME) { $flags += 'WS_THICKFRAME' }
    if ($ex -band $N::WS_EX_TOOLWINDOW) { $flags += 'WS_EX_TOOLWINDOW' }
    if ($ex -band $N::WS_EX_APPWINDOW) { $flags += 'WS_EX_APPWINDOW' }
    if ($ex -band $N::WS_EX_LAYERED) { $flags += 'WS_EX_LAYERED' }
    if ($ex -band $N::WS_EX_TOPMOST) { $flags += 'WS_EX_TOPMOST' }
    if ($ex -band $N::WS_EX_NOACTIVATE) { $flags += 'WS_EX_NOACTIVATE' }
    if ($ex -band $N::WS_EX_NOREDIRECTIONBITMAP) { $flags += 'WS_EX_NOREDIRECTIONBITMAP' }
    Write-Output ("flags      : " + ($flags -join ' | '))
    Write-Output ("DPI        : window={0} awareness={1} (0=unaware 1=system 2=per-monitor)" -f $N::GetDpiForWindow($h), $N::GetAwarenessFromDpiAwarenessContext($N::GetWindowDpiAwarenessContext($h)))
}

function Invoke-MinimizeAll([bool]$undo) {
    $shell = New-Object -ComObject Shell.Application
    if ($undo) { $shell.UndoMinimizeALL() } else { $shell.MinimizeAll() }
    Start-Sleep -Milliseconds 800
    Show-State
}

function Set-NoActivate([bool]$on) {
    $h = Find-MainWindow
    $ex = [int64]$N::GetWindowLongPtr($h, $N::GWL_EXSTYLE)
    $ex = if ($on) { $ex -bor $N::WS_EX_NOACTIVATE } else { $ex -band (-bnot $N::WS_EX_NOACTIVATE) }
    [void]$N::SetWindowLongPtr($h, $N::GWL_EXSTYLE, [IntPtr]$ex)
    [void]$N::SetWindowPos($h, $N::HWND_BOTTOM, 0, 0, 0, 0, ($N::SWP_NOMOVE -bor $N::SWP_NOSIZE -bor $N::SWP_NOACTIVATE -bor $N::SWP_FRAMECHANGED))
    Show-State
}

function Show-DesktopTree {
    $hosts = Get-DesktopHosts
    Write-Output ("Progman: " + (Describe $hosts.Progman))
    foreach ($c in $N::Children($hosts.Progman)) { Write-Output ("  child: " + (Describe $c)) }
    foreach ($w in $hosts.WorkerWs) {
        Write-Output ("WorkerW: " + (Describe $w))
        foreach ($c in $N::Children($w)) { Write-Output ("  child: " + (Describe $c)) }
    }
}

function Show-ZOrder {
    # 从顶层 Z 序最底部往上列出 25 个窗口（桌面相关窗口都在底部）
    $h = $N::GetWindow($N::GetShellWindow(), $N::GW_HWNDLAST)
    $i = 0
    while ($h -ne [IntPtr]::Zero -and $i -lt 25) {
        if ($N::IsWindowVisible($h)) { Write-Output ("z[{0,2}] {1}" -f $i, (Describe $h)); $i++ }
        $h = $N::GetWindow($h, $N::GW_HWNDPREV)
    }
}

function Invoke-ToggleDesktop {
    $shell = New-Object -ComObject Shell.Application
    $shell.ToggleDesktop()
    Start-Sleep -Milliseconds 800
}

function Save-Shot {
    Add-Type -AssemblyName System.Drawing
    $h = Find-MainWindow
    $r = New-Object Probe.Native+RECT
    [void]$N::GetWindowRect($h, [ref]$r)
    $pad = 40
    $x = $r.L - $pad; $y = $r.T - $pad; $w = ($r.R - $r.L) + 2 * $pad; $hh = ($r.B - $r.T) + 2 * $pad
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
    $g.Dispose()
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Output "saved $Out (screen rect $x,$y ${w}x$hh)"
}

function Embed-Child {
    $h = Find-MainWindow
    $hosts = Get-DesktopHosts
    if ($hosts.IconHost -eq [IntPtr]::Zero) { throw '没找到承载 SHELLDLL_DefView 的宿主窗口' }
    $r = New-Object Probe.Native+RECT
    [void]$N::GetWindowRect($h, [ref]$r)
    $hr = New-Object Probe.Native+RECT
    [void]$N::GetWindowRect($hosts.IconHost, [ref]$hr)
    # WS_POPUP -> WS_CHILD（MSDN 要求在 SetParent 前切换）
    $style = [int64]$N::GetWindowLongPtr($h, $N::GWL_STYLE)
    $style = ($style -band (-bnot $N::WS_POPUP)) -bor $N::WS_CHILD
    [void]$N::SetWindowLongPtr($h, $N::GWL_STYLE, [IntPtr]$style)
    $prev = $N::SetParent($h, $hosts.IconHost)
    Write-Output ("SetParent prev=0x{0:X} -> host=0x{1:X}" -f $prev.ToInt64(), $hosts.IconHost.ToInt64())
    # 子窗口坐标相对宿主客户区；宿主一般覆盖整个虚拟屏幕
    $cx = $r.L - $hr.L; $cy = $r.T - $hr.T
    # 排到 DefView 之上：insertAfter = DefView 的前一个同级（即紧贴其上），退化为 HWND_TOP
    [void]$N::SetWindowPos($h, $N::HWND_TOP, $cx, $cy, 0, 0, ($N::SWP_NOSIZE -bor $N::SWP_NOACTIVATE -bor $N::SWP_FRAMECHANGED -bor $N::SWP_SHOWWINDOW))
    Write-Output "moved to host-relative ($cx,$cy)"
    Show-State
}

function Embed-Owner {
    $h = Find-MainWindow
    $hosts = Get-DesktopHosts
    $style = [int64]$N::GetWindowLongPtr($h, $N::GWL_STYLE)
    $style = $style -band (-bnot $N::WS_MINIMIZEBOX)
    [void]$N::SetWindowLongPtr($h, $N::GWL_STYLE, [IntPtr]$style)
    $ex = [int64]$N::GetWindowLongPtr($h, $N::GWL_EXSTYLE)
    $ex = ($ex -bor $N::WS_EX_TOOLWINDOW) -band (-bnot $N::WS_EX_APPWINDOW)
    [void]$N::SetWindowLongPtr($h, $N::GWL_EXSTYLE, [IntPtr]$ex)
    $prev = $N::SetWindowLongPtr($h, $N::GWLP_HWNDPARENT, $hosts.Progman)
    Write-Output ("owner prev=0x{0:X} -> Progman=0x{1:X}" -f $prev.ToInt64(), $hosts.Progman.ToInt64())
    [void]$N::SetWindowPos($h, $N::HWND_BOTTOM, 0, 0, 0, 0, ($N::SWP_NOMOVE -bor $N::SWP_NOSIZE -bor $N::SWP_NOACTIVATE -bor $N::SWP_FRAMECHANGED))
    Show-State
}

function Restore-TopLevel {
    $h = Find-MainWindow
    $r = New-Object Probe.Native+RECT
    [void]$N::GetWindowRect($h, [ref]$r)
    $style = [int64]$N::GetWindowLongPtr($h, $N::GWL_STYLE)
    $style = ($style -band (-bnot $N::WS_CHILD)) -bor $N::WS_POPUP -bor $N::WS_MINIMIZEBOX
    [void]$N::SetWindowLongPtr($h, $N::GWL_STYLE, [IntPtr]$style)
    [void]$N::SetParent($h, [IntPtr]::Zero)
    [void]$N::SetWindowLongPtr($h, $N::GWLP_HWNDPARENT, [IntPtr]::Zero)
    $ex = [int64]$N::GetWindowLongPtr($h, $N::GWL_EXSTYLE)
    $ex = ($ex -band (-bnot $N::WS_EX_TOOLWINDOW)) -bor $N::WS_EX_APPWINDOW
    [void]$N::SetWindowLongPtr($h, $N::GWL_EXSTYLE, [IntPtr]$ex)
    [void]$N::SetWindowPos($h, $N::HWND_TOP, $r.L, $r.T, 0, 0, ($N::SWP_NOSIZE -bor $N::SWP_NOACTIVATE -bor $N::SWP_FRAMECHANGED -bor $N::SWP_SHOWWINDOW))
    Show-State
}

function Invoke-Click {
    $h = Find-MainWindow
    $r = New-Object Probe.Native+RECT
    [void]$N::GetWindowRect($h, [ref]$r)
    $sx = $r.L + $X; $sy = $r.T + $Y
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($sx, $sy)
    Start-Sleep -Milliseconds 100
    if (-not ('Probe.Mouse' -as [type])) {
        Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
namespace Probe { public static class Mouse {
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  public static void Click() { mouse_event(0x2, 0, 0, 0, UIntPtr.Zero); mouse_event(0x4, 0, 0, 0, UIntPtr.Zero); }
} }
'@
    }
    [Probe.Mouse]::Click()
    Write-Output "clicked at screen ($sx,$sy)"
}

function Move-Main {
    $h = Find-MainWindow
    # 顶层窗口：屏幕坐标；子窗口：相对宿主客户区（调用方自己换算）
    [void]$N::SetWindowPos($h, [IntPtr]::Zero, $X, $Y, 0, 0, ($N::SWP_NOSIZE -bor $N::SWP_NOZORDER -bor $N::SWP_NOACTIVATE))
    Show-State
}

function Watch-Toggle {
    # 触发显示桌面后高频采样 1.5s：只要有一次 IsIconic 为真就说明被最小化过（哪怕随后被轮询线程还原）
    $h = Find-MainWindow
    $shell = New-Object -ComObject Shell.Application
    $shell.ToggleDesktop()
    $everIconic = $false; $everHidden = $false; $rects = New-Object System.Collections.Generic.HashSet[string]
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 1500) {
        if ($N::IsIconic($h)) { $everIconic = $true }
        if (-not $N::IsWindowVisible($h)) { $everHidden = $true }
        $r = New-Object Probe.Native+RECT; [void]$N::GetWindowRect($h, [ref]$r)
        [void]$rects.Add("($($r.L),$($r.T))-($($r.R),$($r.B))")
        Start-Sleep -Milliseconds 20
    }
    Write-Output ("everIconic={0} everHidden={1} rectsSeen={2}" -f $everIconic, $everHidden, ($rects -join ' ; '))
    Show-State
}

function Hover-Main {
    $h = Find-MainWindow
    $r = New-Object Probe.Native+RECT
    [void]$N::GetWindowRect($h, [ref]$r)
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point(($r.L + $X), ($r.T + $Y))
    Start-Sleep -Milliseconds 400
    Write-Output ("cursor at screen ({0},{1})" -f ($r.L + $X), ($r.T + $Y))
}

switch ($Action) {
    'minimize-all'   { Invoke-MinimizeAll $false }
    'undo-minimize'  { Invoke-MinimizeAll $true }
    'noactivate-on'  { Set-NoActivate $true }
    'noactivate-off' { Set-NoActivate $false }
    'move'           { Move-Main }
    'watch-toggle'   { Watch-Toggle }
    'hover'          { Hover-Main }
    'state'          { Show-State }
    'desktop-tree'   { Show-DesktopTree }
    'zorder'         { Show-ZOrder }
    'toggle-desktop' { Invoke-ToggleDesktop; Show-State }
    'shot'           { Save-Shot }
    'embed-child'    { Embed-Child }
    'embed-owner'    { Embed-Owner }
    'restore'        { Restore-TopLevel }
    'click'          { Invoke-Click }
}
