//! Windows Shell 调用（COM）。
//!
//! 除 [`known_folder_public_desktop`] 外，所有函数都必须在已初始化 COM STA 的线程上调用
//! （由 `sta.rs` 保证：移动 / 启动 / 定位走 `run_in_new_sta`，图标走 `icon_worker`）。

use super::icon::RawImage;
use super::location::same_path_ci;
use super::ops::{BatchOutcome, MoveJob, MoveOutcome};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use windows::core::{implement, Interface, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IBindCtx, IPersistFile, CLSCTX_ALL, CLSCTX_INPROC_SERVER,
    STGM_READ,
};
use windows::Win32::UI::Shell::{
    FOLDERID_PublicDesktop, FileOperation, IFileOperation, IFileOperationProgressSink,
    IFileOperationProgressSink_Impl, ILCreateFromPathW, ILFree, IShellItem, IShellItemImageFactory,
    IShellLinkW, SHCreateItemFromParsingName, SHGetKnownFolderPath, SHOpenFolderAndSelectItems,
    ShellExecuteExW, ShellLink, FOFX_NOMINIMIZEBOX, FOF_NOCONFIRMMKDIR, FOF_RENAMEONCOLLISION,
    KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, SIGDN_FILESYSPATH,
    SIIGBF,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// 公共桌面路径（`SHGetKnownFolderPath(FOLDERID_PublicDesktop)`，不需要 COM）
pub(crate) fn known_folder_public_desktop() -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_PublicDesktop, KF_FLAG_DEFAULT, HANDLE::default())
            .ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path
    }
}

/// IShellItem → 文件系统路径
fn shell_item_path(item: &IShellItem) -> Option<PathBuf> {
    unsafe {
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path
    }
}

fn shell_item(path: &Path) -> windows::core::Result<IShellItem> {
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None::<&IBindCtx>) }
}

/// HRESULT 的中文说明（常见码给固定文案，其余用系统消息 + 十六进制）
pub(crate) fn describe_hresult(hr: i32) -> String {
    match hr as u32 {
        0x8007_0002 | 0x8007_0003 => "文件不存在".to_string(),
        0x8007_0005 => "没有权限".to_string(),
        0x8007_0020 | 0x8007_0021 => "正被其他程序占用".to_string(),
        0x8007_0070 => "磁盘空间不足".to_string(),
        0x8007_00CE | 0x8007_006F => "路径过长".to_string(),
        0x8007_00B7 | 0x8007_0050 => "目标位置已存在同名项".to_string(),
        code => {
            let message = windows::core::Error::from(HRESULT(hr)).message();
            let message = message.trim();
            if message.is_empty() {
                format!("操作失败（0x{:08X}）", code)
            } else {
                format!("{}（0x{:08X}）", message, code)
            }
        }
    }
}

/// `PostMoveItem` 回调记录：（HRESULT, 移动后的新路径）
type MoveSlot = Arc<Mutex<Option<(i32, Option<PathBuf>)>>>;

/// 单个移动任务的进度回调：只记录顶层项的 `PostMoveItem`（跨卷移动文件夹时子项也会回调）
#[implement(IFileOperationProgressSink)]
struct MoveSink {
    source: PathBuf,
    slot: MoveSlot,
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for MoveSink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hrresult: HRESULT) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreRenameItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psznewname: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psznewname: &PCWSTR,
        _hrrename: HRESULT,
        _psinewlycreated: Option<&IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _dwflags: u32,
        psiitem: Option<&IShellItem>,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
        hrmove: HRESULT,
        psinewlycreated: Option<&IShellItem>,
    ) -> windows::core::Result<()> {
        let item_path = psiitem.and_then(shell_item_path);
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        let is_top_level = match &item_path {
            Some(path) => same_path_ci(path, &self.source),
            // 拿不到源路径时，只接受第一次回调
            None => slot.is_none(),
        };
        if is_top_level {
            *slot = Some((hrmove.0, psinewlycreated.and_then(shell_item_path)));
        }
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
        _hrcopy: HRESULT,
        _psinewlycreated: Option<&IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreDeleteItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _hrdelete: HRESULT,
        _psinewlycreated: Option<&IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreNewItem(
        &self,
        _dwflags: u32,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _dwflags: u32,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
        _psztemplatename: &PCWSTR,
        _dwfileattributes: u32,
        _hrnew: HRESULT,
        _psinewitem: Option<&IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _iworktotal: u32, _iworksofar: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

/// 用一个 `IFileOperation` 批量执行移动任务（必须在 STA 线程上调用）。
///
/// 标志：`FOF_RENAMEONCOLLISION`（兜底：绝不覆盖）| `FOF_NOCONFIRMMKDIR` | `FOFX_NOMINIMIZEBOX`。
/// 不带 `FOF_NOERRORUI`：权限不足时由 Shell 弹"需要管理员权限才能移动"对话框 → UAC，
/// 文件被占用时弹"文件正在使用"对话框（重试 / 跳过 / 取消），行为与资源管理器拖放一致。
pub(crate) fn perform_moves(jobs: &[MoveJob], owner_hwnd: Option<isize>) -> BatchOutcome {
    let mut batch = BatchOutcome {
        items: vec![MoveOutcome::default(); jobs.len()],
        ..Default::default()
    };
    let op: IFileOperation = match unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) } {
        Ok(op) => op,
        Err(e) => {
            batch.perform_error = Some(format!("无法创建文件操作对象: {}", e.message()));
            return batch;
        }
    };
    let flags = FOF_RENAMEONCOLLISION | FOF_NOCONFIRMMKDIR | FOFX_NOMINIMIZEBOX;
    if let Err(e) = unsafe { op.SetOperationFlags(flags) } {
        batch.perform_error = Some(format!("设置文件操作参数失败: {}", e.message()));
        return batch;
    }
    if let Some(hwnd) = owner_hwnd.filter(|h| *h != 0) {
        if let Err(e) = unsafe { op.SetOwnerWindow(HWND(hwnd as *mut _)) } {
            log::warn!("[fence] SetOwnerWindow 失败: {}", e.message());
        }
    }

    let mut slots = Vec::with_capacity(jobs.len());
    let mut queued = 0usize;
    for (index, job) in jobs.iter().enumerate() {
        let slot = Arc::new(Mutex::new(None));
        slots.push(slot.clone());
        let item = match shell_item(&job.source) {
            Ok(item) => item,
            Err(e) => {
                batch.items[index].error = Some(format!("无法访问源文件: {}", e.message()));
                continue;
            }
        };
        let dest = match shell_item(&job.dest_dir) {
            Ok(dest) => dest,
            Err(e) => {
                batch.items[index].error = Some(format!("无法访问目标文件夹: {}", e.message()));
                continue;
            }
        };
        let sink: IFileOperationProgressSink = MoveSink {
            source: job.source.clone(),
            slot,
        }
        .into();
        let name = HSTRING::from(job.dest_name.as_str());
        match unsafe { op.MoveItem(&item, &dest, &name, &sink) } {
            Ok(()) => queued += 1,
            Err(e) => batch.items[index].error = Some(format!("无法加入移动队列: {}", e.message())),
        }
    }
    if queued == 0 {
        return batch;
    }

    if let Err(e) = unsafe { op.PerformOperations() } {
        // 用户取消时这里也会返回错误，逐项结论以回调为准
        batch.perform_error = Some(describe_hresult(e.code().0));
    }
    batch.aborted = unsafe { op.GetAnyOperationsAborted() }
        .map(|b| b.as_bool())
        .unwrap_or(false);

    for (index, slot) in slots.into_iter().enumerate() {
        let recorded = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some((hr, new_path)) = recorded {
            let outcome = &mut batch.items[index];
            outcome.hr = Some(hr);
            outcome.new_path = new_path;
            if hr < 0 {
                outcome.error = Some(describe_hresult(hr));
            }
        }
    }
    batch
}

/// 需要把工作目录设为所在目录的可执行文件类型
fn needs_working_dir(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .is_some_and(|ext| matches!(ext.as_str(), "exe" | "com" | "bat" | "cmd"))
}

/// 解析 `.lnk` 的目标文件系统路径（已展开环境变量）。IDList 型快捷方式（UWP / 商店应用等）没有
/// 文件路径，返回 `None`。只读取快捷方式，不做 Resolve（不会触发搜索）。必须在 STA 线程上调用。
pub(crate) fn shortcut_target(lnk: &Path) -> Option<PathBuf> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        file.Load(&HSTRING::from(lnk), STGM_READ).ok()?;
        let mut buf = vec![0u16; 32_768];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len])))
    }
}

fn is_shortcut_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("lnk"))
}

fn describe_launch_error(path: &Path, err: &windows::core::Error) -> String {
    let code = err.code().0 as u32;
    // 目标失效的 .lnk 在 SEE_MASK_FLAG_NO_UI 下会先静默尝试解析（约 3 秒），失败后返回
    // ERROR_CANCELLED 或 FILE_NOT_FOUND；读取快捷方式里的目标路径确认是否真的失效
    if is_shortcut_path(path) && matches!(code, 0x8007_04C7 | 0x8007_0002 | 0x8007_0003) {
        if let Some(target) = shortcut_target(path) {
            if std::fs::symlink_metadata(&target).is_err() {
                return "快捷方式指向的目标不存在，可能已被移动或卸载".to_string();
            }
        }
    }
    match code {
        0x8007_0002 | 0x8007_0003 => "文件不存在".to_string(),
        0x8007_04C7 => "已取消".to_string(),
        0x8007_0483 => "没有与该类型文件关联的程序".to_string(),
        0x8007_0005 => "没有权限打开".to_string(),
        0x8007_00C1 => "不是有效的程序".to_string(),
        _ => format!("打开失败：{}", describe_hresult(err.code().0)),
    }
}

/// `ShellExecuteExW` 默认动作打开（必须在 STA 线程上调用）。
///
/// `SEE_MASK_FLAG_NO_UI`：目标失效的快捷方式不弹 Shell 的"快捷方式存在问题"对话框，改为返回错误
/// （UAC 提示不受影响）；`SEE_MASK_NOASYNC`：调用所在的临时线程随后就会退出，必须等执行完成。
pub(crate) fn launch(path: &Path) -> Result<(), String> {
    let file = HSTRING::from(path);
    let directory = needs_working_dir(path)
        .then(|| path.parent().map(HSTRING::from))
        .flatten();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpFile: PCWSTR(file.as_ptr()),
        lpDirectory: directory
            .as_ref()
            .map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| describe_launch_error(path, &e))
}

/// 在资源管理器中打开所在文件夹并选中该项（必须在 STA 线程上调用）
pub(crate) fn reveal(path: &Path) -> Result<(), String> {
    unsafe {
        let pidl = ILCreateFromPathW(&HSTRING::from(path));
        if pidl.is_null() {
            return Err("无法解析该路径".to_string());
        }
        // cidl = 0 时 pidlFolder 是单个项目的完整 PIDL：打开其所在文件夹并选中它
        let result = SHOpenFolderAndSelectItems(pidl, None, 0);
        ILFree(Some(pidl));
        result.map_err(|e| format!("打开所在位置失败：{}", describe_hresult(e.code().0)))
    }
}

/// `IShellItemImageFactory::GetImage` 提取图标 / 缩略图（必须在 STA 线程上调用）
pub(crate) fn extract_image(path: &Path, size_px: u32, flags: SIIGBF) -> Result<RawImage, String> {
    unsafe {
        let item = shell_item(path).map_err(|e| format!("无法读取图标: {}", e.message()))?;
        let factory: IShellItemImageFactory = item
            .cast()
            .map_err(|e| format!("无法读取图标: {}", e.message()))?;
        let size = SIZE {
            cx: size_px as i32,
            cy: size_px as i32,
        };
        let hbitmap = factory
            .GetImage(size, flags)
            .map_err(|e| format!("提取图标失败: {}", describe_hresult(e.code().0)))?;
        let result = bitmap_to_bgra(hbitmap);
        let _ = DeleteObject(hbitmap);
        result
    }
}

/// HBITMAP → 32 位 BGRA 像素（`GetDIBits` 传负高度，得到自上而下的行序）
unsafe fn bitmap_to_bgra(hbitmap: HBITMAP) -> Result<RawImage, String> {
    let mut bitmap = BITMAP::default();
    let got = GetObjectW(
        hbitmap,
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bitmap as *mut BITMAP as *mut _),
    );
    if got == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight == 0 {
        return Err("图标位图无效".to_string());
    }
    let width = bitmap.bmWidth;
    let height = bitmap.bmHeight.abs();
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; width as usize * height as usize * 4];
    let hdc = CreateCompatibleDC(HDC::default());
    let lines = GetDIBits(
        hdc,
        hbitmap,
        0,
        height as u32,
        Some(bgra.as_mut_ptr() as *mut _),
        &mut info,
        DIB_RGB_COLORS,
    );
    let _ = DeleteDC(hdc);
    if lines != height {
        return Err("读取图标像素失败".to_string());
    }
    Ok(RawImage {
        width: width as u32,
        height: height as u32,
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_dir_only_for_executables() {
        assert!(needs_working_dir(Path::new(r"C:\a\b.EXE")));
        assert!(needs_working_dir(Path::new(r"C:\a\run.bat")));
        assert!(!needs_working_dir(Path::new(r"C:\a\b.lnk")));
        assert!(!needs_working_dir(Path::new(r"C:\a\folder")));
    }

    #[test]
    fn hresult_descriptions() {
        assert_eq!(describe_hresult(0x8007_0020_u32 as i32), "正被其他程序占用");
        assert_eq!(describe_hresult(0x8007_0005_u32 as i32), "没有权限");
        assert!(describe_hresult(0x8027_0000_u32 as i32).contains("0x80270000"));
    }

    #[test]
    fn shortcut_target_reads_raw_target() {
        use super::super::folder::test_support::TempDir;
        let tmp = TempDir::new("lnk-target");
        let lnk = tmp.path().join("坏的.lnk");
        let target = tmp.path().join("不存在").join("app.exe");
        let (lnk2, target2) = (lnk.clone(), target.clone());
        let read = super::super::sta::run_in_new_sta("t-lnk", move || unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            link.SetPath(&HSTRING::from(target2.as_path()))
                .map_err(|e| e.to_string())?;
            let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
            file.Save(&HSTRING::from(lnk2.as_path()), true)
                .map_err(|e| e.to_string())?;
            let missing = lnk2.with_file_name("不存在的快捷方式.lnk");
            Ok((shortcut_target(&lnk2), shortcut_target(&missing)))
        })
        .unwrap();
        assert_eq!(read.0, Some(target));
        assert_eq!(read.1, None);
    }

    #[test]
    fn public_desktop_resolves() {
        let public = known_folder_public_desktop().expect("公共桌面应可解析");
        assert!(public.is_absolute());
    }
}
