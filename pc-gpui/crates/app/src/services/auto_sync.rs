//! 自动同步定时器（旧 `MainView.startAutoSync` / `runAutoSync`）。**W1 实现**，基于 [`super::sync::run_sync`]：
//! 间隔 `max(1, syncInterval || 15)` 分钟、`SyncMode::Smart { full: false }`；`AppState.syncing` 时跳过本轮；
//! 失败段只提示一次（`AppState.auto_sync_failing`），忙碌错误不算失败；设置窗口关闭 / 同步设置变化后重建。

use gpui_kit::App;

/// 按当前同步设置（重新）启动自动同步（`run()` 与设置窗口关闭后调用）
pub fn start(_cx: &mut App) {}
