# e2e 拖放源：一个 WinForms 小窗口，在它上面按下鼠标即开始真实的 OLE 拖放（DoDragDrop，FileDrop 数据），
# 由 run_e2e.py 用真实鼠标输入把它拖到收纳箱窗口上松开。拖放结果（DragDropEffects）追加写到 -ResultFile。
param(
    [Parameter(Mandatory = $true)][string]$Path,
    [string]$Title = '__mini_todo_e2e_dragsrc',
    [int]$X = 40, [int]$Y = 300, [int]$W = 220, [int]$H = 160,
    [string]$ResultFile = ''
)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class DragDpi { [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx); }
'@
[void][DragDpi]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))
$form = New-Object System.Windows.Forms.Form
$form.Text = $Title
$form.StartPosition = 'Manual'
$form.Left = $X; $form.Top = $Y; $form.Width = $W; $form.Height = $H
$form.TopMost = $true
$form.BackColor = [System.Drawing.Color]::FromArgb(255, 180, 220, 250)
$label = New-Object System.Windows.Forms.Label
$label.Text = "drag: $([System.IO.Path]::GetFileName($Path))"
$label.Dock = 'Fill'
$label.TextAlign = 'MiddleCenter'
$form.Controls.Add($label)
$handler = {
    $files = New-Object System.Collections.Specialized.StringCollection
    [void]$files.Add($Path)
    $data = New-Object System.Windows.Forms.DataObject
    $data.SetFileDropList($files)
    $effect = $form.DoDragDrop($data, [System.Windows.Forms.DragDropEffects]::Copy -bor [System.Windows.Forms.DragDropEffects]::Move)
    if ($ResultFile) { Add-Content -Path $ResultFile -Value "effect=$effect" }
}
$label.Add_MouseDown($handler)
$form.Add_MouseDown($handler)
[System.Windows.Forms.Application]::Run($form)
