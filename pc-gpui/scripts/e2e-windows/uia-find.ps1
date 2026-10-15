# 用 UI Automation 找一个元素的屏幕矩形（物理像素）。默认在桌面（Progman / WorkerW 下的图标列表）里找；
# 输出 "x,y,w,h"，找不到输出 notfound 并以 1 退出。
# 用法：powershell -NoProfile -File uia-find.ps1 -Name <名字> [-Hwnd <句柄>] [-TimeoutSec 10]
param([Parameter(Mandatory = $true)][string]$Name, [long]$Hwnd = 0, [int]$TimeoutSec = 10)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class UiaDpi { [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx); }
'@
[void][UiaDpi]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))
$A = [System.Windows.Automation.AutomationElement]
$scope = [System.Windows.Automation.TreeScope]
$cond = New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $Name)
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
    $roots = @()
    if ($Hwnd) { $roots += $A::FromHandle([IntPtr]$Hwnd) }
    else {
        foreach ($k in $A::RootElement.FindAll($scope::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
            if ($k.Current.ClassName -in @('Progman', 'WorkerW')) { $roots += $k }
        }
    }
    foreach ($r in $roots) {
        $el = $r.FindFirst($scope::Descendants, $cond)
        if ($el) {
            $b = $el.Current.BoundingRectangle
            if ($b.Width -gt 0) { "{0},{1},{2},{3}" -f [int]$b.X, [int]$b.Y, [int]$b.Width, [int]$b.Height; exit 0 }
        }
    }
    Start-Sleep -Milliseconds 500
}
"notfound"
# 诊断：桌面图标列表里有哪些项
$names = @()
foreach ($k in $A::RootElement.FindAll($scope::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
    if ($k.Current.ClassName -in @('Progman', 'WorkerW')) {
        $items = $k.FindAll($scope::Descendants, (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::ListItem)))
        foreach ($i in $items) { $names += $i.Current.Name }
        $names += "[$($k.Current.ClassName) 0x$('{0:X}' -f $k.Current.NativeWindowHandle) items=$($items.Count)]"
    }
}
"desktop items: $($names -join ' | ')"
$hide = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced' -Name HideIcons -ErrorAction SilentlyContinue).HideIcons
"HideIcons=$hide"
exit 1
