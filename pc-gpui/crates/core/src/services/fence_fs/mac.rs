//! macOS 文件层原始调用（`services::fence_fs` 的 macOS 后端）：移动、图标、打开、在访达中显示、iCloud 判定、
//! 文件标志（隐藏 / 锁定）。
//!
//! 这个文件**不依赖本 crate 的其它模块**（不 `use super::` / `crate::`）：入参出参只有路径、字符串、bool 与普通结构。
//! 这样 Linux / Windows 上也能把它原样编进 `pc-gpui/scripts/mac-check` 做 `--target aarch64-apple-darwin` 的类型检查
//! （core 整体依赖 bundled SQLite，只能在 macOS 上编译）。同一份源码要能按 edition 2021（core）与 2024（mac-check）编译：
//! extern 块写 `unsafe extern "C"`，unsafe 操作一律放进显式的 `unsafe {}`。
//!
//! - 全部是**阻塞**函数，可以（也应该）在后台线程调用，从不需要 `MainThreadMarker`：NSFileManager / NSWorkspace /
//!   NSImage / NSBitmapImageRep / NSGraphicsContext 都不是 `MainThreadOnly`；Apple 文档写明 `iconForFile:`、
//!   `openURL:configuration:completionHandler:`、`activateFileViewerSelectingURLs:` 可在任意线程调用。
//!   用到 Foundation / AppKit 的函数都包在 `autoreleasepool` 里（Rust 线程没有自动释放池）。
//! - **绝不覆盖**：移动用 `renamex_np(RENAME_EXCL)`（内核持 rename 锁原子判重，目标已存在返回 `EEXIST`），同名时换调用方给的
//!   下一个候选名重试；跨卷（`EXDEV`）或卷不支持 `RENAME_EXCL`（`ENOTSUP`）才交给 `NSFileManager moveItemAtURL:toURL:error:`
//!   （目标已存在时报错不覆盖；跨卷用 copyfile EXCL 复制再删源）。**不要用 `std::fs::rename` / `rename(2)`**：
//!   目标已存在时它会静默替换。
//! - 错误信息是面向用户的中文。
//!
//! 背景资料：`.trellis/tasks/10-16-macos-fences/research/mac-file-ops.md`；契约见 `.trellis/spec/backend/desktop-fences.md`。

use std::ffi::CString;
use std::io;
use std::os::macos::fs::MetadataExt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapFormat, NSBitmapImageFileType, NSBitmapImageRep, NSColorSpace, NSCompositingOperation,
    NSDeviceRGBColorSpace, NSGraphicsContext, NSImageInterpolation, NSRunningApplication,
    NSWorkspace, NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{
    NSArray, NSCocoaErrorDomain, NSDictionary, NSError, NSFileManager, NSFileWriteFileExistsError,
    NSNumber, NSPOSIXErrorDomain, NSPoint, NSRect, NSSize, NSString,
    NSURLBookmarkResolutionOptions, NSURLIsAliasFileKey, NSURLIsUbiquitousItemKey,
    NSUnderlyingErrorKey, NSURL,
};

/// 同一目标目录里最多尝试多少个候选名（`a.txt`、`a (2).txt` … `a (64).txt`）
pub const MAX_NAME_ATTEMPTS: u32 = 64;

/// 打开项目时等待 LaunchServices 回调的最长时间；超时按成功处理（应用可能还在启动）
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

// `sys/resource.h`（libc 0.2.180 没有导出 setiopolicy_np 及这些常量）
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
const IOPOL_SCOPE_THREAD: libc::c_int = 1;
const IOPOL_MATERIALIZE_DATALESS_FILES_ON: libc::c_int = 2;

unsafe extern "C" {
    fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
    fn getiopolicy_np(iotype: libc::c_int, scope: libc::c_int) -> libc::c_int;
}

/// 移动一项的结果
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveResult {
    /// 移动后的实际完整路径；`None` 表示没有移动
    pub final_path: Option<PathBuf>,
    /// 失败时的 errno（`EPERM`、`EACCES`……）；取不到具体 errno 的失败为 `None`
    pub errno: Option<i32>,
    /// 失败原因（中文）；成功但源未能删除时是提示
    pub message: Option<String>,
    /// 跨卷移动：已完整复制到目标位置，但删除源失败（两边各有一份，绝不删目标）
    pub source_left: bool,
}

impl MoveResult {
    fn moved(dest: PathBuf) -> Self {
        MoveResult {
            final_path: Some(dest),
            ..Default::default()
        }
    }

    fn failed(errno: Option<i32>, message: String) -> Self {
        MoveResult {
            errno,
            message: Some(message),
            ..Default::default()
        }
    }
}

fn c_path(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

fn last_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// 是否带 `UF_HIDDEN`（`chflags hidden`，访达不显示）：对应 Windows 的隐藏属性
pub fn has_hidden_flag(meta: &std::fs::Metadata) -> bool {
    meta.st_flags() & libc::UF_HIDDEN != 0
}

/// 是否被锁定（访达「显示简介 → 已锁定」= `uchg`；或 `schg` / 仅追加标志）：这类项目不能删除 / 改名 / 移动
pub fn is_locked(path: &Path) -> bool {
    const LOCK_FLAGS: u32 =
        libc::UF_IMMUTABLE | libc::SF_IMMUTABLE | libc::UF_APPEND | libc::SF_APPEND;
    std::fs::symlink_metadata(path)
        .map(|meta| meta.st_flags() & LOCK_FLAGS != 0)
        .unwrap_or(false)
}

/// macOS ≤ 13 的 iCloud 云端文件在本地只有隐藏的占位文件 `.名字.icloud`：真实名字的路径不存在（移动会得到 `ENOENT`）
pub fn has_legacy_icloud_stub(path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let stub = parent.join(format!(".{}.icloud", name.to_string_lossy()));
    std::fs::symlink_metadata(stub).is_ok()
}

/// 该项目是否由 iCloud 云盘（或其它 File Provider，如「桌面与文稿」同步）管理（`NSURLIsUbiquitousItemKey`）。
///
/// 移出 iCloud 管理的桌面等于把它从 iCloud 删除（其它设备上也会消失），所以要在**移动之前**判断。
/// 只读这个项目自身的属性（拖放已授权该项目），不会触发「桌面文件夹」隐私授权框。取不到时按 `false`
pub fn is_ubiquitous_item(path: &Path) -> bool {
    autoreleasepool(|_| {
        let Some(url) = NSURL::from_file_path(path) else {
            return false;
        };
        let mut value: Option<Retained<AnyObject>> = None;
        // SAFETY: NSURLIsUbiquitousItemKey 的值类型是 NSNumber（BOOL），`value` 是可写的 Option<Retained<AnyObject>>
        let ok = unsafe { url.getResourceValue_forKey_error(&mut value, NSURLIsUbiquitousItemKey) };
        ok.is_ok()
            && value
                .and_then(|v| v.downcast::<NSNumber>().ok())
                .is_some_and(|n| n.boolValue())
    })
}

/// 允许本线程在访问"只在云端"（dataless）的文件时触发下载，返回原策略（交给 [`restore_materialize_policy`]）。
///
/// iCloud 云盘（macOS 14+ 走 File Provider）把逐出的文件变成 APFS dataless 文件；rename 它到同步域之外时内核先让
/// File Provider 下载完再改名（不会留下坏文件），策略禁止下载时返回 `EDEADLK`。显式打开，行为不依赖进程默认值
pub fn allow_materialize_on_this_thread() -> libc::c_int {
    // SAFETY: 只读写调用线程自己的 I/O 策略，参数是 sys/resource.h 的常量
    unsafe {
        let old = getiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_THREAD,
        );
        if setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_THREAD,
            IOPOL_MATERIALIZE_DATALESS_FILES_ON,
        ) != 0
        {
            log::warn!(
                "[fence] 设置 dataless 文件下载策略失败: {}",
                io::Error::last_os_error()
            );
        }
        old
    }
}

/// 还原 [`allow_materialize_on_this_thread`] 之前的策略
pub fn restore_materialize_policy(old: libc::c_int) {
    if old >= 0 {
        // SAFETY: 同上
        unsafe {
            setiopolicy_np(
                IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                IOPOL_SCOPE_THREAD,
                old,
            );
        }
    }
}

/// errno → 中文原因。`source` / `dest_dir` 用来区分"已锁定"与"隐私权限被拒"（两者都是 `EPERM`）
pub fn describe_errno(source: &Path, dest_dir: &Path, errno: i32) -> String {
    match errno {
        libc::EPERM => {
            let locked =
                is_locked(source) || source.parent().is_some_and(is_locked) || is_locked(dest_dir);
            if locked {
                "项目或所在文件夹已锁定（访达「显示简介」里勾选了「已锁定」），请先解锁".to_string()
            } else {
                "macOS 拒绝访问该位置：请在「系统设置 → 隐私与安全性 → 文件与文件夹」中允许 mini-todo 访问相应文件夹"
                    .to_string()
            }
        }
        libc::EACCES => "没有权限".to_string(),
        // 源还在：是目标文件夹不见了（规划之后被删 / 改名）
        libc::ENOENT if std::fs::symlink_metadata(source).is_ok() => "目标文件夹不存在".to_string(),
        libc::ENOENT if has_legacy_icloud_stub(source) => {
            "该文件只在 iCloud 中，请先在访达中下载该文件".to_string()
        }
        libc::ENOENT => "文件不存在".to_string(),
        libc::EDEADLK | libc::ETIMEDOUT => {
            "该文件只在 iCloud 中、未能下载，请先在访达中下载该文件".to_string()
        }
        libc::ENOSPC => "磁盘空间不足".to_string(),
        libc::EROFS => "目标位置是只读的".to_string(),
        libc::ENAMETOOLONG => "路径过长".to_string(),
        libc::EBUSY => "正被系统占用".to_string(),
        libc::EEXIST => "目标位置同名项目过多".to_string(),
        code => format!("移动失败（{}）", io::Error::from_raw_os_error(code)),
    }
}

/// 把 `source` 移到 `dest_dir` 下，绝不覆盖。
///
/// 先试 `first_name`；目标已存在（`EEXIST`）时依次试 `next_name(2)`、`next_name(3)`……（最多 [`MAX_NAME_ATTEMPTS`] 个名字），
/// 去重规则（` (2)` 插在扩展名前、包按文件处理）由调用方决定。阻塞；跨卷移动大文件 / 下载云端文件时可能很久。
/// 符号链接移动的是链接本身。
pub fn move_item(
    source: &Path,
    dest_dir: &Path,
    first_name: &str,
    next_name: &dyn Fn(u32) -> String,
) -> MoveResult {
    let Some(src) = c_path(source) else {
        return MoveResult::failed(None, "路径含有非法字符".to_string());
    };
    let mut name = first_name.to_string();
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let dest = dest_dir.join(&name);
        let Some(dst) = c_path(&dest) else {
            return MoveResult::failed(None, "路径含有非法字符".to_string());
        };
        // SAFETY: 两个指针都是有效的 NUL 结尾 C 字符串，调用期间存活
        let rc = unsafe { libc::renamex_np(src.as_ptr(), dst.as_ptr(), libc::RENAME_EXCL) };
        if rc == 0 {
            return MoveResult::moved(dest);
        }
        let errno = last_errno();
        match errno {
            libc::EEXIST => {}
            // 跨卷；或卷不支持 RENAME_EXCL（部分网络卷）：交给 NSFileManager（目标已存在时报错、不覆盖）
            libc::EXDEV | libc::ENOTSUP => match move_with_file_manager(source, &dest, dest_dir) {
                FileManagerMove::Exists => {}
                FileManagerMove::Done(result) => return result,
            },
            _ => {
                return MoveResult::failed(Some(errno), describe_errno(source, dest_dir, errno));
            }
        }
        name = next_name(attempt + 1);
    }
    MoveResult::failed(
        Some(libc::EEXIST),
        describe_errno(source, dest_dir, libc::EEXIST),
    )
}

enum FileManagerMove {
    /// 目标已存在：换下一个名字
    Exists,
    Done(MoveResult),
}

/// `NSFileManager moveItemAtURL:toURL:error:`：同卷是 lstat + rename，跨卷是 copyfile（EXCL，逐项不覆盖）+ 删源。
/// 复制中途失败时 Foundation 删掉目标里的半成品；复制完成但删源失败时不回滚（两边各一份，不丢文件）
fn move_with_file_manager(source: &Path, dest: &Path, dest_dir: &Path) -> FileManagerMove {
    autoreleasepool(|_| {
        let (Some(src_url), Some(dst_url)) =
            (NSURL::from_file_path(source), NSURL::from_file_path(dest))
        else {
            return FileManagerMove::Done(MoveResult::failed(None, "路径含有非法字符".to_string()));
        };
        // 不设 delegate：defaultManager 可跨线程使用；新建一个实例也一样
        let manager = NSFileManager::defaultManager();
        match manager.moveItemAtURL_toURL_error(&src_url, &dst_url) {
            Ok(()) => FileManagerMove::Done(MoveResult::moved(dest.to_path_buf())),
            Err(error) => {
                if is_cocoa_error(&error, NSFileWriteFileExistsError) {
                    return FileManagerMove::Exists;
                }
                let errno = underlying_posix_code(&error);
                if errno == Some(libc::EEXIST) {
                    return FileManagerMove::Exists;
                }
                let dest_exists = std::fs::symlink_metadata(dest).is_ok();
                let source_exists = std::fs::symlink_metadata(source).is_ok();
                if dest_exists && source_exists {
                    // 复制已完成、删源失败：绝不删目标
                    return FileManagerMove::Done(MoveResult {
                        final_path: Some(dest.to_path_buf()),
                        errno,
                        message: Some("已复制到目标位置，但原位置的项目未能删除".to_string()),
                        source_left: true,
                    });
                }
                let message = match errno {
                    Some(code) => describe_errno(source, dest_dir, code),
                    None => format!("移动失败（{}）", error.localizedDescription()),
                };
                FileManagerMove::Done(MoveResult::failed(errno, message))
            }
        }
    })
}

fn is_cocoa_error(error: &NSError, code: isize) -> bool {
    // SAFETY: Foundation 导出的常量 NSString
    let cocoa = unsafe { NSCocoaErrorDomain };
    error.code() == code && &*error.domain() == cocoa
}

/// NSError 对应的 POSIX errno：本身是 `NSPOSIXErrorDomain`，或 `NSUnderlyingErrorKey` 里的那个
/// （`NSError.underlyingErrors` 要 macOS 11.3+，最低支持 11，所以读 userInfo）
fn underlying_posix_code(error: &NSError) -> Option<i32> {
    // SAFETY: Foundation 导出的常量 NSString
    let posix = unsafe { NSPOSIXErrorDomain };
    if &*error.domain() == posix {
        return i32::try_from(error.code()).ok();
    }
    // SAFETY: 同上
    let key = unsafe { NSUnderlyingErrorKey };
    let info = error.userInfo();
    let underlying: Retained<AnyObject> = info.objectForKey(key)?;
    let underlying = underlying.downcast::<NSError>().ok()?;
    if &*underlying.domain() == posix {
        i32::try_from(underlying.code()).ok()
    } else {
        None
    }
}

/// 图标 PNG：`NSWorkspace iconForFile:` 画进自建的 `px × px` 位图（1 point = 1 像素，sRGB，预乘 alpha），
/// 再由 AppKit 编码成 PNG。返回 PNG 字节，边长恰好是 `px`（夹到 16..=1024）。
///
/// 不改 `iconForFile:` 返回的图片（可能是共享缓存对象，`setSize:` 属于修改），只往自己新建的位图里画；
/// 后台线程绘图要自己 `flushGraphics`。替身 / 符号链接带左下角小箭头（与访达一致）。路径不存在时
/// `iconForFile:` 也会返回通用图标，调用方先确认文件存在
pub fn icon_png(path: &Path, px: u32) -> Result<Vec<u8>, String> {
    let px = px.clamp(16, 1024) as isize;
    let side = px as f64;
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let image = workspace.iconForFile(&NSString::from_str(&path.to_string_lossy()));

        // SAFETY: planes 传 NULL → AppKit 自己分配并持有像素缓冲；8 bit × 4 通道（含 alpha）、非平面是合法组合；
        // bitmapFormat 为空 = 预乘、alpha 在后（CoreGraphics 位图上下文只支持预乘）
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                ptr::null_mut(),
                px,
                px,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                NSBitmapFormat::empty(),
                0,
                0,
            )
        }
        .ok_or_else(|| "提取图标失败：无法创建位图".to_string())?;
        let rep = rep
            .bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace())
            .unwrap_or(rep);
        rep.setSize(NSSize::new(side, side));

        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)
            .ok_or_else(|| "提取图标失败：无法创建绘图上下文".to_string())?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&context));
        context.setImageInterpolation(NSImageInterpolation::High);
        image.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side)),
            NSRect::ZERO, // NSZeroRect = 整张图
            NSCompositingOperation::SourceOver,
            1.0,
        );
        context.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();

        // SAFETY: 空属性字典，类型与选择器要求一致
        let data = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or_else(|| "提取图标失败：PNG 编码失败".to_string())?;
        Ok(data.to_vec())
    })
}

/// 替身（访达的 alias 文件）是否失效：是替身且解析不出原身 → `true`。不是替身 / 不确定 → `false`
fn is_broken_alias(path: &Path) -> bool {
    autoreleasepool(|_| {
        let Some(url) = NSURL::from_file_path(path) else {
            return false;
        };
        let mut value: Option<Retained<AnyObject>> = None;
        // SAFETY: NSURLIsAliasFileKey 的值类型是 NSNumber（BOOL）
        let ok = unsafe { url.getResourceValue_forKey_error(&mut value, NSURLIsAliasFileKey) };
        let is_alias = ok.is_ok()
            && value
                .and_then(|v| v.downcast::<NSNumber>().ok())
                .is_some_and(|n| n.boolValue());
        if !is_alias {
            return false;
        }
        // 不弹界面、不挂载网络卷：只判断原身还在不在
        let options = NSURLBookmarkResolutionOptions::WithoutUI
            | NSURLBookmarkResolutionOptions::WithoutMounting;
        NSURL::URLByResolvingAliasFileAtURL_options_error(&url, options).is_err()
    })
}

/// 打开项目（双击）：`NSWorkspace openURL:configuration:completionHandler:`（默认程序打开文件、启动 `.app`、
/// 在访达中打开文件夹；替身 / 符号链接由 LaunchServices 解析）。
///
/// 先检查替身 / 符号链接是否失效（LaunchServices 可能弹访达风格的"找不到原身"对话框），再等 LaunchServices 的回调拿
/// 错误信息；最多等 [`LAUNCH_TIMEOUT`]，超时按成功处理。阻塞
pub fn launch(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "文件不存在".to_string())?;
    if meta.file_type().is_symlink() && std::fs::metadata(path).is_err() {
        return Err("符号链接指向的目标不存在，可能已被移动或删除".to_string());
    }
    if is_broken_alias(path) {
        return Err("替身指向的原身不存在，可能已被移动或删除".to_string());
    }
    let (tx, rx) = mpsc::channel::<Option<String>>();
    autoreleasepool(|_| {
        let url = NSURL::from_file_path(path).ok_or_else(|| "路径含有非法字符".to_string())?;
        let configuration = NSWorkspaceOpenConfiguration::configuration();
        configuration.setActivates(true);
        // 回调在 LaunchServices 的私有队列上执行，只往 channel 里发结果
        let handler = RcBlock::new(
            move |_app: *mut NSRunningApplication, error: *mut NSError| {
                // SAFETY: 回调期间 error 要么为 NULL，要么指向有效的 NSError
                let message =
                    unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
                let _ = tx.send(message);
            },
        );
        NSWorkspace::sharedWorkspace().openURL_configuration_completionHandler(
            &url,
            &configuration,
            Some(&handler),
        );
        Ok::<(), String>(())
    })?;
    match rx.recv_timeout(LAUNCH_TIMEOUT) {
        Ok(Some(message)) => Err(format!("打开失败：{message}")),
        Ok(None) | Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
        // 回调没执行就被释放（不应发生）：没有错误可报
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(()),
    }
}

/// 在访达中显示并选中该项（右键「在访达中显示」）：`activateFileViewerSelectingURLs:`。该方法没有错误返回，先确认路径存在
pub fn reveal(path: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err("文件不存在".to_string());
    }
    autoreleasepool(|_| {
        let url = NSURL::from_file_path(path).ok_or_else(|| "路径含有非法字符".to_string())?;
        NSWorkspace::sharedWorkspace()
            .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        Ok(())
    })
}
