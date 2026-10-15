# 桌面收纳箱窗口 e2e 检查脚本（只读查询 + Shell 动作；收纳箱的增删改走 cdp.mjs 调命令 / 点 DOM）
# 用法：pwsh -NoProfile -File fence.ps1 -Step <list|json|zorder|win-d|win-d-key|win-m|restart-explorer|memory|probe-start|probe-state|probe-stop>
#
# 只认路径匹配 -ExePath 的 mini-todo 进程（默认 debug 构建），绝不会碰到正在运行的安装版。
# 收纳箱窗口 = 该进程里与主窗口（标题 Mini Todo）同一窗口类、标题不是 Mini Todo 的顶层窗口
# （测试期间不要打开设置 / 编辑器窗口，否则它们也会被列出来）。
[CmdletBinding()]
param(
    [ValidateSet('list', 'json', 'zorder', 'win-d', 'win-d-key', 'win-m', 'restart-explorer', 'memory', 'probe-start', 'probe-state', 'probe-stop')]
    [string]$Step = 'list',
    [string]$ExePath = 'target\debug\mini-todo.exe',
    # probe-start：探针窗口（普通应用窗口）的位置与尺寸（物理像素）
    [int]$X = 200, [int]$Y = 200, [int]$W = 640, [int]$H = 420,
    [string]$ProbeTitle = '__fence_e2e_probe',
    [int]$WatchMs = 1500
)
$ErrorActionPreference = 'Stop'

if (-not ('E2E.Fence' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace E2E {
  public static class Fence {
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
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    public const int GWL_STYLE = -16, GWL_EXSTYLE = -20, GWLP_HWNDPARENT = -8;
    public const uint GW_HWNDLAST = 1, GW_HWNDPREV = 3, GA_ROOT = 2;
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
$N = [E2E.Fence]
[void]$N::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
$ProbePidFile = Join-Path $env:TEMP 'mini-todo-fence-e2e-probe.pid'

function Get-DevPids {
    $procs = Get-Process -Name 'mini-todo' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "*$ExePath*" }
    if (-not $procs) { throw "没有找到路径匹配 '$ExePath' 的 mini-todo 进程" }
    return @($procs | ForEach-Object { [uint32]$_.Id })
}

function Get-AppWindows {
    $pids = Get-DevPids
    $all = @($N::TopLevel() | Where-Object { $pids -contains $N::Pid($_) })
    $main = $all | Where-Object { $N::Text($_) -eq 'Mini Todo' } | Select-Object -First 1
    if (-not $main) { throw '进程存在但没找到标题为 Mini Todo 的主窗口' }
    $cls = $N::Class($main)
    $fences = @($all | Where-Object { $_ -ne $main -and $N::Class($_) -eq $cls -and $N::Text($_) -ne '' })
    [pscustomobject]@{ Main = $main; Fences = $fences }
}

function Get-ZIndex([IntPtr]$target) {
    # 从 Z 序最底（Progman）往上数可见顶层窗口，返回 target 的序号；-1 表示没找到 / 不可见
    $h = $N::GetWindow($N::GetShellWindow(), $N::GW_HWNDLAST)
    $i = 0
    while ($h -ne [IntPtr]::Zero) {
        if ($N::IsWindowVisible($h)) { if ($h -eq $target) { return $i }; $i++ }
        $h = $N::GetWindow($h, $N::GW_HWNDPREV)
    }
    return -1
}

function Get-RectText([IntPtr]$h) {
    $r = New-Object E2E.Fence+RECT; [void]$N::GetWindowRect($h, [ref]$r)
    return "($($r.L),$($r.T))-($($r.R),$($r.B))"
}

function Get-WindowState([IntPtr]$h, [string]$kind) {
    $style = [int64]$N::GetWindowLongPtr($h, $N::GWL_STYLE)
    $ex = [int64]$N::GetWindowLongPtr($h, $N::GWL_EXSTYLE)
    $owner = [int64]$N::GetWindowLongPtr($h, $N::GWLP_HWNDPARENT)
    $progman = $N::GetShellWindow().ToInt64()
    [pscustomobject]@{
        kind           = $kind
        hwnd           = ('0x{0:X}' -f $h.ToInt64())
        title          = $N::Text($h)
        rect           = (Get-RectText $h)
        visible        = $N::IsWindowVisible($h)
        iconic         = $N::IsIconic($h)
        owner          = ('0x{0:X}' -f $owner)
        ownerIsProgman = ($owner -ne 0 -and $owner -eq $progman)
        zIndex         = (Get-ZIndex $h)
        minimizeBox    = [bool]($style -band $N::WS_MINIMIZEBOX)
        thickFrame     = [bool]($style -band $N::WS_THICKFRAME)
        toolWindow     = [bool]($ex -band $N::WS_EX_TOOLWINDOW)
        appWindow      = [bool]($ex -band $N::WS_EX_APPWINDOW)
        topmost        = [bool]($ex -band $N::WS_EX_TOPMOST)
        noActivate     = [bool]($ex -band $N::WS_EX_NOACTIVATE)
        dpi            = $N::GetDpiForWindow($h)
    }
}

function Get-AllStates {
    $w = Get-AppWindows
    $states = @(Get-WindowState $w.Main 'main')
    foreach ($f in $w.Fences) { $states += Get-WindowState $f 'fence' }
    $probe = Find-Probe
    if ($probe -ne [IntPtr]::Zero) { $states += Get-WindowState $probe 'probe' }
    return $states
}

function Find-Probe {
    if (-not (Test-Path $ProbePidFile)) { return [IntPtr]::Zero }
    $probePid = [uint32](Get-Content $ProbePidFile)
    foreach ($h in $N::TopLevel()) { if ($N::Pid($h) -eq $probePid -and $N::Text($h) -eq $ProbeTitle) { return $h } }
    return [IntPtr]::Zero
}

function Get-CenterHit([IntPtr]$h) {
    $r = New-Object E2E.Fence+RECT; [void]$N::GetWindowRect($h, [ref]$r)
    $p = New-Object E2E.Fence+POINT; $p.X = [int](($r.L + $r.R) / 2); $p.Y = [int](($r.T + $r.B) / 2)
    return $N::GetAncestor($N::WindowFromPoint($p), $N::GA_ROOT)
}

function Watch-ShellAction([scriptblock]$action) {
    $w = Get-AppWindows
    $targets = @(@{ kind = 'main'; h = $w.Main })
    foreach ($f in $w.Fences) { $targets += @{ kind = 'fence'; h = $f } }
    $probe = Find-Probe
    if ($probe -ne [IntPtr]::Zero) { $targets += @{ kind = 'probe'; h = $probe } }
    $track = @{}
    foreach ($t in $targets) { $track[$t.h] = @{ iconic = $false; hidden = $false; rects = New-Object System.Collections.Generic.HashSet[string] } }
    & $action
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $WatchMs) {
        foreach ($t in $targets) {
            $s = $track[$t.h]
            if ($N::IsIconic($t.h)) { $s.iconic = $true }
            if (-not $N::IsWindowVisible($t.h)) { $s.hidden = $true }
            [void]$s.rects.Add((Get-RectText $t.h))
        }
        Start-Sleep -Milliseconds 20
    }
    foreach ($t in $targets) {
        $s = $track[$t.h]
        $hit = Get-CenterHit $t.h
        [pscustomobject]@{
            kind          = $t.kind
            hwnd          = ('0x{0:X}' -f $t.h.ToInt64())
            title         = $N::Text($t.h)
            everIconic    = $s.iconic
            everHidden    = $s.hidden
            iconicNow     = $N::IsIconic($t.h)
            visibleNow    = $N::IsWindowVisible($t.h)
            rectsSeen     = ($s.rects -join ' ; ')
            zIndexAfter   = (Get-ZIndex $t.h)
            centerHitSelf = ($hit -eq $t.h)
            centerHit     = ('0x{0:X} [{1}] {2}' -f $hit.ToInt64(), $N::Class($hit), $N::Text($hit))
        }
    }
}

function Send-WinD {
    $VK_LWIN = 0x5B; $VK_D = 0x44; $KEYUP = 0x2
    $N::keybd_event($VK_LWIN, 0, 0, [UIntPtr]::Zero)
    $N::keybd_event($VK_D, 0, 0, [UIntPtr]::Zero)
    $N::keybd_event($VK_D, 0, $KEYUP, [UIntPtr]::Zero)
    $N::keybd_event($VK_LWIN, 0, $KEYUP, [UIntPtr]::Zero)
}

function Show-ZOrder {
    $pids = Get-DevPids
    $h = $N::GetWindow($N::GetShellWindow(), $N::GW_HWNDLAST)
    $i = 0
    while ($h -ne [IntPtr]::Zero -and $i -lt 14) {
        if ($N::IsWindowVisible($h)) {
            $mark = if ($pids -contains $N::Pid($h)) { '*' } else { ' ' }
            Write-Output ("{0}z[{1,2}] 0x{2:X} [{3}] '{4}' pid={5}" -f $mark, $i, $h.ToInt64(), $N::Class($h), $N::Text($h), $N::Pid($h)); $i++
        }
        $h = $N::GetWindow($h, $N::GW_HWNDPREV)
    }
}

function Get-Memory {
    # 开发版进程树（mini-todo.exe + 它拉起的 msedgewebview2.exe 及其子进程）的私有内存 / 工作集
    $rootPid = (Get-DevPids)[0]
    $procs = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name, CommandLine
    $byParent = @{}
    foreach ($p in $procs) { if (-not $byParent.ContainsKey($p.ParentProcessId)) { $byParent[$p.ParentProcessId] = @() }; $byParent[$p.ParentProcessId] += $p }
    $tree = New-Object System.Collections.Generic.List[object]
    $queue = New-Object System.Collections.Generic.Queue[uint32]; $queue.Enqueue([uint32]$rootPid)
    $tree.Add(($procs | Where-Object ProcessId -eq $rootPid))
    while ($queue.Count -gt 0) {
        $cur = $queue.Dequeue()
        foreach ($c in @($byParent[$cur])) { if ($c) { $tree.Add($c); $queue.Enqueue([uint32]$c.ProcessId) } }
    }
    $rows = foreach ($p in $tree) {
        $gp = Get-Process -Id $p.ProcessId -ErrorAction SilentlyContinue
        if (-not $gp) { continue }
        $type = if ($p.CommandLine -match '--type=([\w-]+)') { $Matches[1] } elseif ($p.Name -eq 'msedgewebview2.exe') { 'browser' } else { 'host' }
        $udf = if ($p.CommandLine -match '--user-data-dir="?([^" ]+)') { $Matches[1] } else { '' }
        [pscustomobject]@{ pid = $p.ProcessId; name = $p.Name; type = $type; privateMB = [math]::Round($gp.PrivateMemorySize64 / 1MB, 1); workingSetMB = [math]::Round($gp.WorkingSet64 / 1MB, 1); userDataDir = $udf }
    }
    $rows | Format-Table -AutoSize | Out-String -Width 220 | Write-Output
    $sumPrivate = ($rows | Measure-Object privateMB -Sum).Sum
    $sumWs = ($rows | Measure-Object workingSetMB -Sum).Sum
    $webview = $rows | Where-Object name -eq 'msedgewebview2.exe'
    Write-Output ("processes={0} webview2={1} renderers={2} totalPrivateMB={3:N1} totalWorkingSetMB={4:N1} webview2PrivateMB={5:N1}" -f `
        @($rows).Count, @($webview).Count, @($rows | Where-Object type -eq 'renderer').Count, $sumPrivate, $sumWs, ($webview | Measure-Object privateMB -Sum).Sum)
}

switch ($Step) {
    'list'   { Get-AllStates | Format-Table kind, hwnd, title, rect, visible, iconic, ownerIsProgman, zIndex, minimizeBox, thickFrame, toolWindow, appWindow, topmost, dpi -AutoSize | Out-String -Width 240 | Write-Output }
    'json'   { Get-AllStates | ConvertTo-Json -Compress }
    'zorder' { Show-ZOrder }
    'win-d'  { Watch-ShellAction { (New-Object -ComObject Shell.Application).ToggleDesktop() } | Format-Table -AutoSize | Out-String -Width 260 | Write-Output }
    'win-d-key' { Watch-ShellAction { Send-WinD } | Format-Table -AutoSize | Out-String -Width 260 | Write-Output }
    'win-m'  { Watch-ShellAction { (New-Object -ComObject Shell.Application).MinimizeAll() } | Format-Table -AutoSize | Out-String -Width 260 | Write-Output }
    'memory' { Get-Memory }
    'probe-start' {
        $script = Join-Path $PSScriptRoot 'probe-window.ps1'
        $p = Start-Process pwsh -ArgumentList @('-NoProfile', '-WindowStyle', 'Hidden', '-File', $script, '-Title', $ProbeTitle, '-X', $X, '-Y', $Y, '-W', $W, '-H', $H) -PassThru
        Set-Content -Path $ProbePidFile -Value $p.Id
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        while ($sw.Elapsed.TotalSeconds -lt 20 -and (Find-Probe) -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 200 }
        $h = Find-Probe
        if ($h -eq [IntPtr]::Zero) { throw 'probe window did not appear' }
        Get-WindowState $h 'probe' | Format-List
    }
    'probe-state' {
        $h = Find-Probe
        if ($h -eq [IntPtr]::Zero) { throw 'no probe window' }
        Get-WindowState $h 'probe' | Format-List
        Write-Output ('foreground: 0x{0:X} probeIsForeground={1}' -f $N::GetForegroundWindow().ToInt64(), ($N::GetForegroundWindow() -eq $h))
    }
    'probe-stop' {
        if (Test-Path $ProbePidFile) {
            $probePid = [int](Get-Content $ProbePidFile)
            Stop-Process -Id $probePid -Force -ErrorAction SilentlyContinue
            Remove-Item $ProbePidFile -Force
            Write-Output "probe $probePid stopped"
        }
    }
    'restart-explorer' {
        $wins = Get-AppWindows
        $targets = @($wins.Main) + $wins.Fences
        foreach ($t in $targets) { Write-Output ('before: 0x{0:X} {1} owner=0x{2:X}' -f $t.ToInt64(), $N::Text($t), [int64]$N::GetWindowLongPtr($t, $N::GWLP_HWNDPARENT)) }
        Write-Output ('progman before: 0x{0:X}' -f $N::GetShellWindow().ToInt64())
        Stop-Process -Name explorer -Force
        Start-Sleep -Milliseconds 1500
        if (-not (Get-Process -Name explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        while ($sw.Elapsed.TotalSeconds -lt 20 -and $N::GetShellWindow() -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 200 }
        Write-Output ('progman back after {0:N1}s: 0x{1:X}' -f $sw.Elapsed.TotalSeconds, $N::GetShellWindow().ToInt64())
        $sw2 = [System.Diagnostics.Stopwatch]::StartNew()
        $pending = @($wins.Fences)
        $done = @{}
        while ($sw2.Elapsed.TotalSeconds -lt 10 -and $pending.Count -gt 0) {
            $shell = $N::GetShellWindow().ToInt64()
            foreach ($f in @($pending)) {
                $owner = [int64]$N::GetWindowLongPtr($f, $N::GWLP_HWNDPARENT)
                if ($N::IsWindow($f) -and $owner -ne 0 -and $owner -eq $shell) { $done[$f] = $sw2.Elapsed.TotalSeconds; $pending = @($pending | Where-Object { $_ -ne $f }) }
            }
            Start-Sleep -Milliseconds 100
        }
        foreach ($f in $wins.Fences) {
            $alive = $N::IsWindow($f)
            $t = if ($done.ContainsKey($f)) { '{0:N1}s' -f $done[$f] } else { 'NOT reattached' }
            Write-Output ('fence 0x{0:X} alive={1} reattached={2}' -f $f.ToInt64(), $alive, $t)
        }
        Write-Output ('main alive={0}' -f $N::IsWindow($wins.Main))
        Get-AllStates | Format-Table kind, hwnd, title, rect, visible, iconic, ownerIsProgman, zIndex, toolWindow, appWindow -AutoSize | Out-String -Width 240 | Write-Output
    }
}
