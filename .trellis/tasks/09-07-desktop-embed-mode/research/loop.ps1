# 桌面 -> 普通 -> 固定 -> 桌面 循环切换，每步断言 Win32 状态 + 进程存活
[CmdletBinding()]
param([int]$Rounds = 5, [int]$GapMs = 1200)
$ErrorActionPreference = 'Stop'
$research = 'D:\Git\mini-todo\.trellis\tasks\09-07-desktop-embed-mode\research'
Set-Location $research

function Click-Mode([string]$title) {
    $r = node cdp.mjs "(() => { const b = [...document.querySelectorAll('button[title]')].find(x => x.title === '$title'); if (!b) return 'NO BUTTON ' + [...document.querySelectorAll('button[title]')].map(x => x.title).join('|'); b.click(); return 'clicked'; })()"
    if ($r -ne 'clicked') { throw "click '$title' failed: $r" }
}
function Get-St { pwsh -NoProfile -File .\e2e.ps1 -Step json | ConvertFrom-Json }
function Assert-Mode([string]$mode, $st, [string]$rect0) {
    $ok = $true; $why = @()
    if ($st.rect -ne $rect0) { $ok = $false; $why += "rect moved $($st.rect)" }
    if ($st.iconic) { $ok = $false; $why += 'iconic' }
    switch ($mode) {
        'desktop' {
            if (-not $st.ownerIsProgman) { $ok = $false; $why += "owner=$($st.owner)" }
            if ($st.zIndex -ne 1) { $ok = $false; $why += "z=$($st.zIndex)" }
            if ($st.minimizeBox) { $ok = $false; $why += 'minimizeBox' }
            if (-not $st.toolWindow) { $ok = $false; $why += 'no toolWindow' }
            if ($st.appWindow) { $ok = $false; $why += 'appWindow' }
            if ($st.topmost) { $ok = $false; $why += 'topmost' }
        }
        'normal' {
            if ($st.owner -ne '0x0') { $ok = $false; $why += "owner=$($st.owner)" }
            if ($st.zIndex -le 1) { $ok = $false; $why += "z=$($st.zIndex)" }
            if (-not $st.minimizeBox) { $ok = $false; $why += 'no minimizeBox' }
            if ($st.toolWindow) { $ok = $false; $why += 'toolWindow' }
            if ($st.topmost) { $ok = $false; $why += 'topmost' }
        }
        'fixed' {
            if ($st.owner -ne '0x0') { $ok = $false; $why += "owner=$($st.owner)" }
            if ($st.zIndex -le 1) { $ok = $false; $why += "z=$($st.zIndex)" }
            if (-not $st.minimizeBox) { $ok = $false; $why += 'no minimizeBox' }
            if (-not $st.toolWindow) { $ok = $false; $why += 'no toolWindow' }
            if ($st.appWindow) { $ok = $false; $why += 'appWindow' }
        }
    }
    $fe = node cdp.mjs "JSON.stringify([...document.querySelectorAll('button[title]')].map(x => x.title).filter(t => /桌面|固定/.test(t)))"
    $expectFe = @{ desktop = '["固定窗口","退出桌面模式"]'; normal = '["固定窗口","桌面模式"]'; fixed = '["取消固定","桌面模式"]' }[$mode]
    if ($fe -ne $expectFe) { $ok = $false; $why += "frontend buttons $fe" }
    $db = python "$PSScriptRoot\dbq.py" "select value from settings where key='is_desktop'"
    $dbFixed = python "$PSScriptRoot\dbq.py" "select value from settings where key='is_fixed'"
    $expectDb = @{ desktop = "('true',)|('false',)"; normal = "('false',)|('false',)"; fixed = "('false',)|('true',)" }[$mode]
    if ("$db|$dbFixed" -ne $expectDb) { $ok = $false; $why += "db is_desktop=$db is_fixed=$dbFixed" }
    return [pscustomobject]@{ mode = $mode; ok = $ok; why = ($why -join '; '); z = $st.zIndex; owner = $st.owner; hwnd = $st.hwnd }
}

$start = Get-St
$rect0 = $start.rect; $hwnd0 = $start.hwnd; $pid0 = $start.pid
Write-Output "start: hwnd=$hwnd0 pid=$pid0 rect=$rect0 owner=$($start.owner) z=$($start.zIndex)"
if (-not $start.ownerIsProgman) { throw 'loop expects to start in desktop mode' }

$results = @()
for ($i = 1; $i -le $Rounds; $i++) {
    # 奇数轮：桌面 -> 普通 -> 固定 -> 桌面；偶数轮：桌面 -> 固定（直切） -> 普通 -> 桌面
    $seq = if ($i % 2 -eq 1) { @(@('退出桌面模式', 'normal'), @('固定窗口', 'fixed'), @('桌面模式', 'desktop')) }
           else { @(@('固定窗口', 'fixed'), @('取消固定', 'normal'), @('桌面模式', 'desktop')) }
    foreach ($step in $seq) {
        Click-Mode $step[0]
        Start-Sleep -Milliseconds $GapMs
        if (-not (Get-Process -Id $pid0 -ErrorAction SilentlyContinue)) { Write-Output "round $i step $($step[0]): PROCESS DIED"; $results += [pscustomobject]@{ round = $i; mode = $step[1]; ok = $false; why = 'process died' }; break }
        $st = Get-St
        $a = Assert-Mode $step[1] $st $rect0
        $results += [pscustomobject]@{ round = $i; mode = $a.mode; ok = $a.ok; why = $a.why; z = $a.z; owner = $a.owner }
        Write-Output ("round {0} {1,-8} ok={2} z={3} owner={4} {5}" -f $i, $a.mode, $a.ok, $a.z, $a.owner, $a.why)
    }
}
$failed = @($results | Where-Object { -not $_.ok })
Write-Output ("TOTAL steps={0} failed={1} hwndSame={2} processAlive={3}" -f $results.Count, $failed.Count, ((Get-St).hwnd -eq $hwnd0), [bool](Get-Process -Id $pid0 -ErrorAction SilentlyContinue))
