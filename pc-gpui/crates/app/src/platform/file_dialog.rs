//! 带标题 / 扩展名过滤器 / 默认文件名的文件对话框（设置页导入 / 导出；GPUI 自带的对话框没有过滤器）。
//!
//! - Windows：`rfd`（系统 `IFileSaveDialog` / `IFileOpenDialog`），父窗口为打开它的 GPUI 窗口；对话框是模态的，
//!   在**独立线程**里运行（不阻塞 GPUI 主线程的消息循环；跨线程的父窗口照样会被系统禁用）。
//! - 其它平台：GPUI 的 `prompt_for_new_path` / `prompt_for_paths`（没有过滤器与标题；Linux 需要 xdg 桌面门户）。
//! - debug 构建：环境变量 `MINI_TODO_DEBUG_DIALOG_PATH` 非空时两种对话框都直接返回该路径（Xvfb 冒烟测试用，release 忽略）。
//!
//! 结果：`None` = 用户取消（或对话框打不开，已记日志）。

use std::path::PathBuf;

use gpui_kit::{App, Task, Window};

/// 扩展名过滤器（`[{ name: 'ZIP 压缩包', extensions: ['zip'] }]`）
#[derive(Debug, Clone, Copy)]
pub struct FileFilter {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

fn debug_override() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        std::env::var_os("MINI_TODO_DEBUG_DIALOG_PATH")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// 保存对话框
pub fn save_file(
    window: &Window,
    cx: &mut App,
    title: &str,
    default_name: &str,
    filter: FileFilter,
) -> Task<Option<PathBuf>> {
    if let Some(path) = debug_override() {
        log::info!("[dialog] MINI_TODO_DEBUG_DIALOG_PATH → {}", path.display());
        return Task::ready(Some(path));
    }
    imp::save_file(window, cx, title, default_name, filter)
}

/// 打开对话框（单选文件）
pub fn open_file(
    window: &Window,
    cx: &mut App,
    title: &str,
    filter: FileFilter,
) -> Task<Option<PathBuf>> {
    if let Some(path) = debug_override() {
        log::info!("[dialog] MINI_TODO_DEBUG_DIALOG_PATH → {}", path.display());
        return Task::ready(Some(path));
    }
    imp::open_file(window, cx, title, filter)
}

#[cfg(windows)]
mod imp {
    use std::num::NonZeroIsize;
    use std::path::PathBuf;

    use gpui_kit::{App, Task, Window};
    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
        Win32WindowHandle, WindowHandle,
    };

    use super::FileFilter;

    /// 只携带 HWND 的父窗口句柄（可以跨线程传给对话框线程）
    #[derive(Clone, Copy)]
    struct Parent(isize);

    impl HasWindowHandle for Parent {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let hwnd = NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?;
            let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
            // SAFETY: 句柄来自仍然存活的 GPUI 窗口；对话框期间窗口被系统禁用、不会被销毁
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    impl HasDisplayHandle for Parent {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::windows())
        }
    }

    fn run<F>(cx: &mut App, f: F) -> Task<Option<PathBuf>>
    where
        F: FnOnce() -> Option<PathBuf> + Send + 'static,
    {
        let (tx, rx) = futures::channel::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("file-dialog".into())
            .spawn(move || {
                let _ = tx.send(f());
            });
        if let Err(e) = spawned {
            log::error!("[dialog] 无法启动对话框线程: {e}");
            return Task::ready(None);
        }
        cx.background_executor()
            .spawn(async move { rx.await.ok().flatten() })
    }

    pub fn save_file(
        window: &Window,
        cx: &mut App,
        title: &str,
        default_name: &str,
        filter: FileFilter,
    ) -> Task<Option<PathBuf>> {
        let parent = crate::platform::hwnd(window).map(|h| Parent(h.0));
        let (title, name) = (title.to_string(), default_name.to_string());
        run(cx, move || {
            let mut dialog = rfd::FileDialog::new()
                .set_title(title)
                .set_file_name(name)
                .add_filter(filter.name, filter.extensions);
            if let Some(parent) = parent {
                dialog = dialog.set_parent(&parent);
            }
            dialog.save_file()
        })
    }

    pub fn open_file(
        window: &Window,
        cx: &mut App,
        title: &str,
        filter: FileFilter,
    ) -> Task<Option<PathBuf>> {
        let parent = crate::platform::hwnd(window).map(|h| Parent(h.0));
        let title = title.to_string();
        run(cx, move || {
            let mut dialog = rfd::FileDialog::new()
                .set_title(title)
                .add_filter(filter.name, filter.extensions);
            if let Some(parent) = parent {
                dialog = dialog.set_parent(&parent);
            }
            dialog.pick_file()
        })
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    use gpui_kit::{App, PathPromptOptions, Task, Window};

    use super::FileFilter;

    pub fn save_file(
        _window: &Window,
        cx: &mut App,
        _title: &str,
        default_name: &str,
        _filter: FileFilter,
    ) -> Task<Option<PathBuf>> {
        let dir = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(default_name));
        cx.background_executor().spawn(async move {
            match rx.await {
                Ok(Ok(path)) => path,
                Ok(Err(e)) => {
                    log::warn!("[dialog] 保存对话框打开失败: {e}");
                    None
                }
                Err(_) => None,
            }
        })
    }

    pub fn open_file(
        _window: &Window,
        cx: &mut App,
        title: &str,
        _filter: FileFilter,
    ) -> Task<Option<PathBuf>> {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(title.to_string().into()),
        });
        cx.background_executor().spawn(async move {
            match rx.await {
                Ok(Ok(paths)) => paths.and_then(|p| p.into_iter().next()),
                Ok(Err(e)) => {
                    log::warn!("[dialog] 打开对话框失败: {e}");
                    None
                }
                Err(_) => None,
            }
        })
    }
}
