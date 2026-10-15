//! WebDAV 同步编排（标题栏同步按钮、自动同步、设置页「立即同步」/ 强制拉取 / 强制推送共用）与
//! `pc/src/utils/syncReport.ts` 的提示文案（逐字移植）。
//!
//! ```ignore
//! // 标题栏同步按钮（旧 MainView.handleSync，W1）
//! let task = services::sync::run_sync(cx, SyncMode::Smart { full: true });
//! cx.spawn_in(window, async move |_, cx| {
//!     match task.await {
//!         Ok(report) => { /* message::show(kind_of(&report), describe_sync_report(&report)) … */ }
//!         Err(e) if is_sync_busy_error(&e) => { /* message::info("已有同步正在进行，请稍后再试") */ }
//!         Err(e) => { /* message::error_with("同步失败", &e) */ }
//!     }
//! }).detach();
//! ```
//!
//! 自动同步定时器（间隔、失败段只提示一次）由主窗口实现者（W1）基于 [`run_sync`] 实现。

use gpui_kit::{App, Task};
use mini_todo_core::sync::{self, SyncMode, SyncReport, SyncStatus};

use crate::db::spawn_db;
use crate::state::AppState;

/// 执行一次同步（后台线程）：期间 `AppState.syncing = true`（标题栏图标旋转）；成功后
/// [`AppState::apply_sync_report`]（同一份报告只处理一次：本地数据有变化且主窗口没有模态子窗口 →
/// 重新拉待办；`settings_applied` → 重新读设置与运行时偏好）。
///
/// 错误原样返回 core 的文本（`同步正在进行中`、`未配置 WebDAV 服务器`、网络错误……），由调用方按场景提示。
pub fn run_sync(cx: &mut App, mode: SyncMode) -> Task<Result<SyncReport, String>> {
    let state = AppState::global(cx);
    state.update(cx, |state, cx| state.begin_sync(cx));
    let task = spawn_db(cx, move |db| sync::run(db, mode));
    cx.spawn(async move |cx| {
        let result = task.await;
        match &result {
            Ok(report) => log::info!(
                "[sync] 同步完成（{:?}）: {}",
                mode,
                describe_sync_report(report)
            ),
            Err(e) => log::warn!("[sync] 同步失败（{mode:?}）: {e}"),
        }
        state.update(cx, |state, cx| {
            state.end_sync(cx);
            if let Ok(report) = &result {
                state.apply_sync_report(report, cx);
            }
        });
        result
    })
}

/// 本地待办 / 子任务是否被改动（`hasLocalDataChanges`）
pub fn has_local_data_changes(report: &SyncReport) -> bool {
    report.todos_inserted
        + report.todos_updated
        + report.todos_deleted
        + report.subtasks_inserted
        + report.subtasks_updated
        + report.subtasks_deleted
        > 0
}

fn status_text(status: SyncStatus) -> &'static str {
    match status {
        SyncStatus::NoChanges => "已是最新，没有需要同步的更改",
        SyncStatus::Pulled => "已从云端拉取更新",
        SyncStatus::Pushed => "已将本地更改推送到云端",
        SyncStatus::Merged => "已合并本地与云端的更改",
    }
}

/// 同步结果提示（`describeSyncReport`），例：`已合并本地与云端的更改（新增待办 2，更新子任务 1，已应用云端设置）`
pub fn describe_sync_report(report: &SyncReport) -> String {
    let base = status_text(report.status);
    let counters = [
        (report.todos_inserted, "新增待办"),
        (report.todos_updated, "更新待办"),
        (report.todos_deleted, "删除待办"),
        (report.subtasks_inserted, "新增子任务"),
        (report.subtasks_updated, "更新子任务"),
        (report.subtasks_deleted, "删除子任务"),
        (report.images_downloaded, "下载图片"),
        (report.images_uploaded, "上传图片"),
    ];
    let mut parts: Vec<String> = counters
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{label} {n}"))
        .collect();
    if report.settings_applied {
        parts.push("已应用云端设置".to_string());
    }
    if parts.is_empty() {
        base.to_string()
    } else {
        format!("{base}（{}）", parts.join("，"))
    }
}

/// 跳过记录的警告（`describeSkippedRecords`）；没有跳过时 `None`
pub fn describe_skipped_records(report: &SyncReport) -> Option<String> {
    (report.records_skipped > 0).then(|| {
        format!(
            "有 {} 条云端记录无法识别，已跳过（未删除任何本地数据）",
            report.records_skipped
        )
    })
}

/// 错误是否为"已有同步在进行"（`isSyncBusyError`）
pub fn is_sync_busy_error(error: &str) -> bool {
    error.contains("同步正在进行中")
}

/// 同步结果提示的类型：没有变化是 info，其它 success（旧 MainView.handleSync）
pub fn report_message_kind(report: &SyncReport) -> crate::ui::message::MessageKind {
    if report.status == SyncStatus::NoChanges {
        crate::ui::message::MessageKind::Info
    } else {
        crate::ui::message::MessageKind::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_report_like_frontend() {
        let report = SyncReport {
            status: SyncStatus::Merged,
            todos_inserted: 2,
            subtasks_updated: 1,
            settings_applied: true,
            ..Default::default()
        };
        assert_eq!(
            describe_sync_report(&report),
            "已合并本地与云端的更改（新增待办 2，更新子任务 1，已应用云端设置）"
        );
        assert!(has_local_data_changes(&report));

        let none = SyncReport::default();
        assert_eq!(describe_sync_report(&none), "已是最新，没有需要同步的更改");
        assert!(!has_local_data_changes(&none));
        assert_eq!(
            report_message_kind(&none),
            crate::ui::message::MessageKind::Info
        );

        let images = SyncReport {
            status: SyncStatus::Pushed,
            images_uploaded: 3,
            images_downloaded: 1,
            ..Default::default()
        };
        assert_eq!(
            describe_sync_report(&images),
            "已将本地更改推送到云端（下载图片 1，上传图片 3）"
        );
        // 只改了设置：不算本地数据变化
        let settings_only = SyncReport {
            status: SyncStatus::Pulled,
            settings_applied: true,
            ..Default::default()
        };
        assert!(!has_local_data_changes(&settings_only));
        assert_eq!(
            describe_sync_report(&settings_only),
            "已从云端拉取更新（已应用云端设置）"
        );
    }

    #[test]
    fn skipped_records_and_busy_error() {
        let report = SyncReport {
            records_skipped: 4,
            ..Default::default()
        };
        assert_eq!(
            describe_skipped_records(&report).as_deref(),
            Some("有 4 条云端记录无法识别，已跳过（未删除任何本地数据）")
        );
        assert_eq!(describe_skipped_records(&SyncReport::default()), None);
        assert!(is_sync_busy_error("同步正在进行中"));
        assert!(is_sync_busy_error("Error: 同步正在进行中，请稍后"));
        assert!(!is_sync_busy_error("未配置 WebDAV 服务器"));
    }
}
