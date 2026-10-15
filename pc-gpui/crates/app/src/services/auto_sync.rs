//! 自动同步定时器（旧 `MainView.startAutoSync` / `runAutoSync`），基于 [`super::sync::run_sync`]：
//! - 间隔 `max(1, syncInterval || 15)` 分钟，`SyncMode::Smart { full: false }`（core 每第 10 轮自动无条件 GET）；
//! - 已有同步在进行（`AppState.syncing`）时跳过这一轮；
//! - 失败段只提示一次（`AppState.auto_sync_failing`），成功或手动同步成功后复位；"同步正在进行中"不算失败；
//! - 重叠调用只让最后一次生效（代次）：启动、设置窗口关闭、同步设置变化后都会重新 `start`。

use std::time::Duration;

use gpui_kit::{App, Global};
use mini_todo_core::sync::{self, SyncMode};

use super::sync as sync_service;
use crate::db::spawn_db;
use crate::state::AppState;
use crate::ui::message::{self, MessageKind};
use crate::windows::WindowManager;

#[derive(Default)]
struct AutoSync {
    generation: u64,
}

impl Global for AutoSync {}

/// 间隔（分钟）：`max(1, syncInterval || 15)`
pub fn interval_minutes(sync_interval: i32) -> u64 {
    let minutes = if sync_interval == 0 {
        15
    } else {
        sync_interval
    };
    minutes.max(1) as u64
}

/// 按当前同步设置（重新）启动自动同步（`run()` 与设置窗口关闭后调用）
pub fn start(cx: &mut App) {
    let generation = {
        let global = cx.default_global::<AutoSync>();
        global.generation += 1;
        global.generation
    };
    let settings = spawn_db(cx, sync::get_settings);
    cx.spawn(async move |cx| {
        let settings = match settings.await {
            Ok(settings) => settings,
            Err(e) => {
                log::warn!("[sync] 读取同步设置失败，自动同步未启动: {e}");
                return;
            }
        };
        if cx.update(|cx| cx.global::<AutoSync>().generation != generation) {
            return;
        }
        if !settings.auto_sync || settings.webdav_url.trim().is_empty() {
            log::info!("[sync] 自动同步未开启");
            return;
        }
        let interval = Duration::from_secs(interval_minutes(settings.sync_interval) * 60);
        log::info!("[sync] 自动同步每 {} 分钟一次", interval.as_secs() / 60);
        loop {
            cx.background_executor().timer(interval).await;
            if cx.update(|cx| cx.global::<AutoSync>().generation != generation) {
                break;
            }
            cx.update(run_auto_sync);
        }
    })
    .detach();
}

/// 停止自动同步（作废当前定时器）
pub fn stop(cx: &mut App) {
    cx.default_global::<AutoSync>().generation += 1;
}

/// 自动同步的一次执行
fn run_auto_sync(cx: &mut App) {
    if AppState::global(cx).read(cx).syncing {
        return;
    }
    let task = sync_service::run_sync(cx, SyncMode::Smart { full: false });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let state = AppState::global(cx);
            match result {
                Ok(report) => {
                    state.update(cx, |state, _| state.auto_sync_failing = false);
                    if let Some(skipped) = sync_service::describe_skipped_records(&report) {
                        log::warn!("[sync] 自动同步: {skipped}");
                    }
                }
                Err(e) if sync_service::is_sync_busy_error(&e) => {}
                Err(e) => {
                    let first = state.update(cx, |state, _| {
                        let first = !state.auto_sync_failing;
                        state.auto_sync_failing = true;
                        first
                    });
                    if first && let Some(main) = WindowManager::main(cx) {
                        let detail = message::error_detail(&e);
                        let text = if detail.is_empty() {
                            "自动同步失败".to_string()
                        } else {
                            format!("自动同步失败：{detail}")
                        };
                        message::show_in(main, cx, MessageKind::Error, text);
                    }
                }
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_defaults_and_minimum() {
        assert_eq!(interval_minutes(0), 15);
        assert_eq!(interval_minutes(30), 30);
        assert_eq!(interval_minutes(-5), 1);
        assert_eq!(interval_minutes(1), 1);
    }
}
