# e2e 拖放源：一个 WinForms 小窗口，在它上面按下鼠标即开始真实的 OLE 拖放（DoDragDrop），
# 由 run_e2e.py 用真实鼠标输入把它拖到收纳箱窗口上松开。拖放结果（DROPEFFECT）追加写到 -ResultFile。
#
#   -Mode shell     （默认）数据对象由 Shell 生成（SHCreateDataObject，与从资源管理器 / 桌面拖文件相同：CF_HDROP、
#                    Shell IDList、拖动图像等格式齐全）
#   -Mode winforms  .NET DataObject.SetFileDropList（只有 FileDrop 等 .NET 格式）
param(
    [Parameter(Mandatory = $true)][string]$Path,
    [ValidateSet('shell', 'winforms')][string]$Mode = 'shell',
    [string]$Title = '__mini_todo_e2e_dragsrc',
    [int]$X = 40, [int]$Y = 300, [int]$W = 220, [int]$H = 160,
    [string]$ResultFile = ''
)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace E2EDrag {
  [ComImport, Guid("00000121-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  public interface IDropSource {
    [PreserveSig] int QueryContinueDrag(int fEscapePressed, uint grfKeyState);
    [PreserveSig] int GiveFeedback(uint dwEffect);
  }
  public class DropSource : IDropSource {
    public int QueryContinueDrag(int esc, uint keys) {
      if (esc != 0) return 0x00040101;            // DRAGDROP_S_CANCEL
      if ((keys & 1) == 0) return 0x00040100;     // MK_LBUTTON 松开 → DRAGDROP_S_DROP
      return 0;
    }
    public int GiveFeedback(uint effect) { return 0x00040102; } // DRAGDROP_S_USEDEFAULTCURSORS
  }
  public static class Native {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)] public static extern int SHParseDisplayName(string name, IntPtr bindCtx, out IntPtr pidl, uint sfgaoIn, out uint sfgaoOut);
    [DllImport("shell32.dll")] public static extern IntPtr ILClone(IntPtr pidl);
    [DllImport("shell32.dll")] public static extern bool ILRemoveLastID(IntPtr pidl);
    [DllImport("shell32.dll")] public static extern IntPtr ILFindLastID(IntPtr pidl);
    [DllImport("shell32.dll")] public static extern int SHCreateDataObject(IntPtr pidlFolder, uint cidl, IntPtr[] apidl, IntPtr pdtInner, ref Guid riid, out IntPtr ppv);
    [DllImport("ole32.dll")] public static extern int DoDragDrop(IntPtr pDataObj, IntPtr pDropSource, int dwOKEffects, out int pdwEffect);

    public static string ShellDrag(string path) {
      IntPtr full; uint sfgao;
      int hr = SHParseDisplayName(path, IntPtr.Zero, out full, 0, out sfgao);
      if (hr != 0) return "SHParseDisplayName hr=0x" + hr.ToString("X8");
      IntPtr parent = ILClone(full);
      ILRemoveLastID(parent);
      IntPtr child = ILFindLastID(full);
      Guid iid = new Guid("0000010e-0000-0000-C000-000000000046"); // IID_IDataObject
      IntPtr data;
      hr = SHCreateDataObject(parent, 1, new IntPtr[] { child }, IntPtr.Zero, ref iid, out data);
      if (hr != 0) return "SHCreateDataObject hr=0x" + hr.ToString("X8");
      IntPtr src = Marshal.GetComInterfaceForObject(new DropSource(), typeof(IDropSource));
      int effect;
      hr = DoDragDrop(data, src, 7, out effect);   // COPY | MOVE | LINK
      Marshal.Release(src);
      Marshal.Release(data);
      return "hr=0x" + hr.ToString("X8") + " effect=" + effect;
    }
  }
}
'@
[void][E2EDrag.Native]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))
[System.Windows.Forms.Application]::OleRequired() | Out-Null
$form = New-Object System.Windows.Forms.Form
$form.Text = $Title
$form.StartPosition = 'Manual'
$form.Left = $X; $form.Top = $Y; $form.Width = $W; $form.Height = $H
$form.TopMost = $true
$form.BackColor = [System.Drawing.Color]::FromArgb(255, 180, 220, 250)
$label = New-Object System.Windows.Forms.Label
$label.Text = "drag ($Mode): $([System.IO.Path]::GetFileName($Path))"
$label.Dock = 'Fill'
$label.TextAlign = 'MiddleCenter'
$form.Controls.Add($label)
$handler = {
    if ($Mode -eq 'shell') {
        $res = [E2EDrag.Native]::ShellDrag($Path)
    } else {
        $files = New-Object System.Collections.Specialized.StringCollection
        [void]$files.Add($Path)
        $data = New-Object System.Windows.Forms.DataObject
        $data.SetFileDropList($files)
        $res = "effect=" + $form.DoDragDrop($data, [System.Windows.Forms.DragDropEffects]::Copy -bor [System.Windows.Forms.DragDropEffects]::Move)
    }
    if ($ResultFile) { Add-Content -Path $ResultFile -Value "$Mode $res" }
}
$label.Add_MouseDown($handler)
$form.Add_MouseDown($handler)
[System.Windows.Forms.Application]::Run($form)
