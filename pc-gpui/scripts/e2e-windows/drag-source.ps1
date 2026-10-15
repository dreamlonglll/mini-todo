# e2e 拖放源：一个 WinForms 小窗口，在它上面按下鼠标即开始真实的 OLE 拖放（DoDragDrop），
# 由 run_e2e.py 用真实鼠标输入把它拖到收纳箱窗口上松开。拖放结果（DROPEFFECT）追加写到 -ResultFile。
#
#   -Mode shell     （默认）数据对象由文件所在 Shell 文件夹生成（IShellFolder::GetUIObjectOf(IID_IDataObject)，
#                    与从资源管理器 / 桌面拖文件时的数据对象相同：CF_HDROP、Shell IDList 等）
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
  [ComImport, Guid("000214E6-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  public interface IShellFolder {
    void ParseDisplayName(IntPtr hwnd, IntPtr pbc, [MarshalAs(UnmanagedType.LPWStr)] string name, out uint eaten, out IntPtr pidl, ref uint attrs);
    void EnumObjects(IntPtr hwnd, int flags, out IntPtr ppenum);
    void BindToObject(IntPtr pidl, IntPtr pbc, ref Guid riid, out IntPtr ppv);
    void BindToStorage(IntPtr pidl, IntPtr pbc, ref Guid riid, out IntPtr ppv);
    [PreserveSig] int CompareIDs(IntPtr lParam, IntPtr pidl1, IntPtr pidl2);
    void CreateViewObject(IntPtr hwnd, ref Guid riid, out IntPtr ppv);
    void GetAttributesOf(uint cidl, [MarshalAs(UnmanagedType.LPArray)] IntPtr[] apidl, ref uint rgf);
    void GetUIObjectOf(IntPtr hwnd, uint cidl, [MarshalAs(UnmanagedType.LPArray)] IntPtr[] apidl, ref Guid riid, IntPtr reserved, out IntPtr ppv);
    void GetDisplayNameOf(IntPtr pidl, uint flags, IntPtr name);
    void SetNameOf(IntPtr hwnd, IntPtr pidl, [MarshalAs(UnmanagedType.LPWStr)] string name, uint flags, out IntPtr ppidlOut);
  }
  public static class Native {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)] public static extern int SHParseDisplayName(string name, IntPtr bindCtx, out IntPtr pidl, uint sfgaoIn, out uint sfgaoOut);
    [DllImport("shell32.dll")] public static extern IntPtr ILClone(IntPtr pidl);
    [DllImport("shell32.dll")] public static extern bool ILRemoveLastID(IntPtr pidl);
    [DllImport("shell32.dll")] public static extern IntPtr ILFindLastID(IntPtr pidl);
    [DllImport("shell32.dll")] public static extern int SHBindToParent(IntPtr pidl, ref Guid riid, out IntPtr ppv, out IntPtr ppidlLast);
    [DllImport("ole32.dll")] public static extern int DoDragDrop(IntPtr pDataObj, IntPtr pDropSource, int dwOKEffects, out int pdwEffect);

    public static string ShellDrag(string path) {
      IntPtr full; uint sfgao;
      int hr = SHParseDisplayName(path, IntPtr.Zero, out full, 0, out sfgao);
      if (hr != 0) return "SHParseDisplayName hr=0x" + hr.ToString("X8");
      Guid iidFolder = new Guid("000214E6-0000-0000-C000-000000000046");
      IntPtr folderPtr, child;
      hr = SHBindToParent(full, ref iidFolder, out folderPtr, out child);
      if (hr != 0) return "SHBindToParent hr=0x" + hr.ToString("X8");
      var folder = (IShellFolder)Marshal.GetObjectForIUnknown(folderPtr);
      Guid iid = new Guid("0000010e-0000-0000-C000-000000000046"); // IID_IDataObject
      IntPtr data;
      folder.GetUIObjectOf(IntPtr.Zero, 1, new IntPtr[] { child }, ref iid, IntPtr.Zero, out data);
      // 记录数据对象是否提供 CF_HDROP（GPUI 的放置目标只认它）
      var com = (System.Runtime.InteropServices.ComTypes.IDataObject)Marshal.GetObjectForIUnknown(data);
      var fe = new System.Runtime.InteropServices.ComTypes.FORMATETC();
      fe.cfFormat = 15; fe.dwAspect = System.Runtime.InteropServices.ComTypes.DVASPECT.DVASPECT_CONTENT; fe.lindex = -1;
      fe.ptd = IntPtr.Zero; fe.tymed = System.Runtime.InteropServices.ComTypes.TYMED.TYMED_HGLOBAL;
      int q = com.QueryGetData(ref fe);
      IntPtr src = Marshal.GetComInterfaceForObject(new DropSource(), typeof(IDropSource));
      int effect;
      hr = DoDragDrop(data, src, 7, out effect);   // COPY | MOVE | LINK
      Marshal.Release(src);
      Marshal.Release(data);
      return "QueryGetData(CF_HDROP)=0x" + q.ToString("X8") + " DoDragDrop hr=0x" + hr.ToString("X8") + " effect=" + effect;
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
