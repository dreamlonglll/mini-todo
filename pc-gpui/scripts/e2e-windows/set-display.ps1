# e2e 前准备显示环境：把主显示器分辨率调到 -Width x -Height（不支持时取不超过它的最大模式），
# 把桌面背景换成纯色（透明窗口取色对照用：透明像素应透出这个颜色）。
# 用法：pwsh -NoProfile -File set-display.ps1 [-Width 1920] [-Height 1080] [-R 150 -G 40 -B 170]
[CmdletBinding()]
param([int]$Width = 1920, [int]$Height = 1080, [int]$R = 150, [int]$G = 40, [int]$B = 170)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
namespace E2E {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
    public short dmSpecVersion; public short dmDriverVersion; public short dmSize; public short dmDriverExtra;
    public int dmFields; public int dmPositionX; public int dmPositionY; public int dmDisplayOrientation;
    public int dmDisplayFixedOutput; public short dmColor; public short dmDuplex; public short dmYResolution;
    public short dmTTOption; public short dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
    public short dmLogPixels; public int dmBitsPerPel; public int dmPelsWidth; public int dmPelsHeight;
    public int dmDisplayFlags; public int dmDisplayFrequency; public int dmICMMethod; public int dmICMIntent;
    public int dmMediaType; public int dmDitherType; public int dmReserved1; public int dmReserved2;
    public int dmPanningWidth; public int dmPanningHeight;
  }
  public static class Display {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern bool EnumDisplaySettings(string dev, int mode, ref DEVMODE dm);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int ChangeDisplaySettings(ref DEVMODE dm, int flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern bool SystemParametersInfo(int action, int param, string vparam, int winIni);
    [DllImport("user32.dll")] public static extern bool SetSysColors(int n, int[] elements, int[] colors);
    public static DEVMODE New() { var d = new DEVMODE(); d.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE)); return d; }
    public static List<int[]> Modes() {
      var l = new List<int[]>(); var d = New();
      for (int i = 0; EnumDisplaySettings(null, i, ref d); i++) l.Add(new int[] { d.dmPelsWidth, d.dmPelsHeight, d.dmBitsPerPel, d.dmDisplayFrequency });
      return l;
    }
  }
}
'@

$cur = [E2E.Display]::New()
[void][E2E.Display]::EnumDisplaySettings($null, -1, [ref]$cur)
"current mode: $($cur.dmPelsWidth)x$($cur.dmPelsHeight) @$($cur.dmBitsPerPel)bpp"
$modes = [E2E.Display]::Modes() | Where-Object { $_[2] -ge 24 } | ForEach-Object { "$($_[0])x$($_[1])" } | Sort-Object -Unique
"available: $($modes -join ', ')"

$target = $null
if ($modes -contains "${Width}x${Height}") { $target = @($Width, $Height) }
else {
  $best = [E2E.Display]::Modes() | Where-Object { $_[0] -le $Width -and $_[1] -le $Height -and $_[2] -ge 24 } |
    Sort-Object { $_[0] * $_[1] } -Descending | Select-Object -First 1
  if ($best) { $target = @($best[0], $best[1]) }
}
if ($target -and ($target[0] -ne $cur.dmPelsWidth -or $target[1] -ne $cur.dmPelsHeight)) {
  $dm = $cur
  $dm.dmPelsWidth = $target[0]; $dm.dmPelsHeight = $target[1]
  $dm.dmFields = 0x80000 -bor 0x100000   # DM_PELSWIDTH | DM_PELSHEIGHT
  $res = [E2E.Display]::ChangeDisplaySettings([ref]$dm, 0)
  "ChangeDisplaySettings -> $($target[0])x$($target[1]): result $res (0 = success)"
  Start-Sleep -Seconds 2
}
Add-Type -AssemblyName System.Windows.Forms
$vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
"virtual screen now: $($vs.Width)x$($vs.Height)"

# 纯色桌面背景
try {
  Set-ItemProperty -Path 'HKCU:\Control Panel\Colors' -Name Background -Value "$R $G $B"
  Set-ItemProperty -Path 'HKCU:\Control Panel\Desktop' -Name WallPaper -Value ''
  [void][E2E.Display]::SystemParametersInfo(0x0014, 0, '', 3)   # SPI_SETDESKWALLPAPER, SPIF_UPDATEINIFILE|SPIF_SENDCHANGE
  [void][E2E.Display]::SetSysColors(1, @(1), @(($B -shl 16) -bor ($G -shl 8) -bor $R))   # COLOR_DESKTOP
  "desktop background set to rgb($R,$G,$B)"
} catch { "desktop background: $_" }

$explorer = Get-Process explorer -ErrorAction SilentlyContinue
"explorer running: $([bool]$explorer) $(@($explorer).Count)"
"OS: $((Get-CimInstance Win32_OperatingSystem).Caption) $((Get-CimInstance Win32_OperatingSystem).Version)"
