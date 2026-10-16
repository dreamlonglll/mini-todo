# 用 UI Automation 找应用在任务栏上的按钮，输出屏幕矩形（物理像素）。只在两处找：
# - Win11（XAML 任务栏）：Shell_TrayWnd 下 ClassName = Taskbar.TaskListButtonAutomationPeer 的元素，
#   Name 形如 "mini-todo - 1 个运行窗口" / "mini-todo - 1 running window"，AutomationId 形如 "Appid: <AppUserModelID>"；
# - Win10 / Windows Server 的经典任务栏：MSTaskListWClass（运行中的程序列表）下的按钮，Name 多为窗口标题 "Mini Todo"。
# 所以不会匹配到通知区的托盘图标（它的名字是托盘提示，如 "Mini Todo（开发版）"）。
# 匹配条件：Name 以任一 -Prefix 开头（不区分大小写），或 AutomationId = "Appid: <-AppId>"。
# 每个匹配输出一行 "x,y,w,h|<Name>|<ClassName>|<任务栏窗口类名>|<AutomationId>"；
# 找不到输出 notfound 与候选按钮名并以 1 退出。
# 本文件带 UTF-8 BOM：Windows PowerShell 5.1 按系统 ANSI 代码页读无 BOM 的脚本，中文注释的末字节可能把换行吞掉。
# 用法：powershell -NoProfile -File taskbar-button.ps1 -Prefix 'mini-todo','Mini Todo' [-AppId <id>] [-TimeoutSec 10]
param([Parameter(Mandatory = $true)][string[]]$Prefix, [string]$AppId = '', [int]$TimeoutSec = 10)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class UiaDpi { [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx); }
'@
[void][UiaDpi]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))
$A = [System.Windows.Automation.AutomationElement]
$scope = [System.Windows.Automation.TreeScope]
$all = [System.Windows.Automation.Condition]::TrueCondition
$script:Seen = @()

function Test-Match([string]$name, [string]$automationId) {
    if ($AppId -and $automationId -eq "Appid: $AppId") { return $true }
    if (-not $name) { return $false }
    foreach ($p in $Prefix) {
        if ($name.StartsWith($p, [System.StringComparison]::OrdinalIgnoreCase)) { return $true }
    }
    return $false
}

function Find-Buttons {
    $script:Seen = @()
    $hits = @()
    foreach ($bar in $A::RootElement.FindAll($scope::Children, $all)) {
        $barClass = $bar.Current.ClassName
        if ($barClass -notin @('Shell_TrayWnd', 'Shell_SecondaryTrayWnd')) { continue }
        $cands = @()
        # 经典任务栏：只看运行中的程序列表
        $classicList = New-Object System.Windows.Automation.PropertyCondition($A::ClassNameProperty, 'MSTaskListWClass')
        foreach ($list in $bar.FindAll($scope::Descendants, $classicList)) {
            $cands += @($list.FindAll($scope::Descendants, $all))
        }
        # Win11 的 XAML 任务栏
        $xamlButton = New-Object System.Windows.Automation.PropertyCondition($A::ClassNameProperty, 'Taskbar.TaskListButtonAutomationPeer')
        $cands += @($bar.FindAll($scope::Descendants, $xamlButton))
        foreach ($el in $cands) {
            $c = $el.Current
            $script:Seen += "$($c.Name) [$($c.ClassName)]"
            if (-not (Test-Match $c.Name $c.AutomationId)) { continue }
            if ($c.IsOffscreen) { continue }
            $b = $c.BoundingRectangle
            if ($b.IsEmpty -or $b.Width -le 0 -or $b.Height -le 0) { continue }
            $hits += "{0},{1},{2},{3}|{4}|{5}|{6}|{7}" -f [int]$b.X, [int]$b.Y, [int]$b.Width, [int]$b.Height, $c.Name, $c.ClassName, $barClass, $c.AutomationId
        }
    }
    return $hits
}

$sw = [Diagnostics.Stopwatch]::StartNew()
while ($true) {
    $hits = @(Find-Buttons)
    if ($hits.Count -gt 0) { $hits; exit 0 }
    if ($sw.Elapsed.TotalSeconds -ge $TimeoutSec) { break }
    Start-Sleep -Milliseconds 500
}
"notfound"
"candidates: $($script:Seen -join ' | ')"
exit 1
