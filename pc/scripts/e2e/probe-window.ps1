# e2e 探针：一个普通的应用顶层窗口（WinForms），用来验证"收纳箱不盖住其它应用窗口"、
# "两次 Win+D 后其它应用窗口能正常恢复"。由 fence.ps1 -Step probe-start / probe-stop 启停。
param([string]$Title = '__fence_e2e_probe', [int]$X = 200, [int]$Y = 200, [int]$W = 640, [int]$H = 420)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ProbeDpi { [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx); }
'@
[void][ProbeDpi]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))
$form = New-Object System.Windows.Forms.Form
$form.Text = $Title
$form.StartPosition = 'Manual'
$form.Left = $X; $form.Top = $Y; $form.Width = $W; $form.Height = $H
$form.BackColor = [System.Drawing.Color]::FromArgb(255, 245, 222, 179)
[System.Windows.Forms.Application]::Run($form)
