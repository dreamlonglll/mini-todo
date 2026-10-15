//! 桌面收纳箱文件层（纯 Rust 服务，与数据库无关）。
//!
//! 每个收纳箱对应收纳根目录 `%USERPROFILE%\MiniTodo收纳箱\` 下的一个真实文件夹，
//! **文件夹是内容的 source of truth**；数据库只存元数据（由调用方负责）。本模块提供：
//!
//! | 能力 | 入口 |
//! |---|---|
//! | 根目录 / 图标缓存目录 / 桌面路径 | [`fence_root_dir`]、[`icon_cache_dir`]、[`user_desktop_dir`]、[`public_desktop_dir`] |
//! | 标题 → 文件夹名、去重、校验 | [`sanitize_folder_name`]、[`dedupe_name`]、[`is_valid_folder_name`] |
//! | 创建 / 改名 / 删除空文件夹 / 列举 | [`create_fence_folder`]、[`rename_fence_folder`]、[`remove_fence_folder_if_empty`]、[`list_fence_items`] |
//! | 移入 / 移回 / 跨箱 / 批量还原 | [`move_into_fence`]、[`restore_item`]、[`move_between_fences`]、[`restore_all`] |
//! | OneDrive 判定 | [`desktop_is_onedrive_redirected`] |
//! | 打开 / 打开所在位置 | [`launch_item`]、[`reveal_in_explorer`] |
//! | 图标 | [`icon_png`]、[`prune_icon_cache`] |
//! | 目录监听 | [`watch_fence_root`] |
//!
//! ## 线程
//!
//! 需要 COM STA 的操作（`IFileOperation`、`IShellItemImageFactory`、`ShellExecuteExW`、
//! `SHOpenFolderAndSelectItems`）在内部专用 STA 线程执行，对外函数可从任意线程调用，但都是**阻塞**的：
//! 在 Tauri async 命令里用 `tauri::async_runtime::spawn_blocking` 包一层。
//! **不要在主线程（同步命令、`run_on_main_thread`、窗口事件回调）里调用带 `owner_hwnd` 的移动操作**：
//! 系统对话框以该窗口为父窗口，会向主线程同步发消息，主线程阻塞等待时会死锁。
//!
//! ## 平台
//!
//! 文件夹管理、列举、命名、判定、监听全平台可用；移动 / 打开 / 定位 / 图标在非 Windows 上返回
//! `Err("桌面收纳箱仅支持 Windows")`。
//!
//! 契约与线程约束见 `.trellis/spec/backend/desktop-fences.md`。

mod folder;
mod icon;
mod location;
mod naming;
mod open;
mod ops;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
mod sta;
mod watch;

/// 非 Windows 平台上 Shell 相关操作的统一错误信息
#[cfg_attr(windows, allow(dead_code))]
pub(crate) const UNSUPPORTED: &str = "桌面收纳箱仅支持 Windows";

// 契约见 `.trellis/spec/backend/desktop-fences.md`；这里只重新导出集成层（`commands::fence*`）用到的项，
// 其余函数仍可经子模块路径在本模块内使用（纯逻辑都有单元测试）
pub use folder::{
    create_fence_folder, ensure_fence_folder, fence_folder_path, list_fence_folders,
    list_fence_items, remove_fence_folder_if_empty, rename_fence_folder, FenceEntry,
};
pub use icon::{icon_png, prune_icon_cache};
pub use location::{
    desktop_is_onedrive_redirected, fence_root_dir, icon_cache_dir, path_starts_with_ci,
    same_path_ci, user_desktop_dir,
};
pub use naming::is_valid_item_name;
pub use open::{launch_item, reveal_in_explorer};
pub use ops::{
    move_between_fences, move_into_fence, restore_all, restore_item, ItemOpResult, ItemOpStatus,
    RestoreAllResult,
};
pub use watch::{watch_fence_root, FenceChange, FenceWatcher};
