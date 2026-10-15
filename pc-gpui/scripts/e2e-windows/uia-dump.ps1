# 诊断：列出某个窗口的 UI Automation 树（GPUI 经 AccessKit 暴露的节点：类型 / 名字 / 屏幕矩形）。
# 用法：powershell -NoProfile -File uia-dump.ps1 -Hwnd <十进制句柄> [-Max 150]
param([Parameter(Mandatory = $true)][long]$Hwnd, [int]$Max = 150)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Hwnd)
$walker = [System.Windows.Automation.TreeWalker]::RawViewWalker
$count = 0
function Walk($el, $depth) {
    if ($script:count -ge $Max) { return }
    $script:count++
    $c = $el.Current
    $r = $c.BoundingRectangle
    $name = $c.Name
    if ($name.Length -gt 40) { $name = $name.Substring(0, 40) + '…' }
    "{0}{1} '{2}' [{3},{4} {5}x{6}] id={7}" -f ('  ' * $depth), $c.ControlType.ProgrammaticName.Replace('ControlType.', ''), $name, [int]$r.X, [int]$r.Y, [int]$r.Width, [int]$r.Height, $c.AutomationId
    $child = $walker.GetFirstChild($el)
    while ($child -ne $null -and $script:count -lt $Max) {
        Walk $child ($depth + 1)
        $child = $walker.GetNextSibling($child)
    }
}
Walk $root 0
"nodes: $count"
