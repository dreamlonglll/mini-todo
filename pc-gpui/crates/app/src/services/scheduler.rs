//! 提醒调度（旧 `services/notification.rs::start_scheduler`）与系统通知。**W2 实现**。
//!
//! 循环（core README「reminder」）：`sleep(STARTUP_DELAY)` → `catch_up_missed_repeats` → 每整分钟
//! `notification_kind` + `run_due(db, deliver)`；阻塞部分放后台，`deliver` 回前台：
//! System → `cx.show_system_notification(..)`（AppUserModelID 已在 `run()` 里 `set_app_identity`）；
//! App → `windows::notification::open(cx, NotificationContent::from(&reminder))`。

use gpui_kit::App;

/// 启动提醒调度（`run()` 里调用一次）
pub fn start(_cx: &mut App) {}
