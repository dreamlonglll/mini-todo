//! 提醒调度（旧 `services/notification.rs::start_scheduler`）。
//!
//! 循环（与旧版一致）：`sleep(STARTUP_DELAY)` → `catch_up_missed_repeats`（补发错过的重复提醒）→ 每到整分钟
//! `notification_kind` + `run_due`。查询、推进状态与系统通知都在后台线程；应用内通知要建窗口，经通道交给前台
//! 打开并等它回话——窗口没建成返回 `Err`，core 就不推进这条提醒、下一分钟重试（与旧版 `send_app_notification`
//! 返回 `Err` 时一致）。单条提醒出错只记日志，不影响同一轮的其它提醒（core `run_due` 负责）。

use std::time::SystemTime;

use gpui_kit::App;
use mini_todo_core::reminder::{self, NotificationKind, Reminder};

use crate::db::Db;
use crate::services::notify;
use crate::windows::notification::{self, NotificationContent};

/// 交给前台打开应用内通知窗口的请求
struct OpenRequest {
    content: NotificationContent,
    reply: async_channel::Sender<Result<(), String>>,
}

/// 发出一条提醒（后台线程调用；应用内通知阻塞等待前台建窗结果）
fn deliver(
    requests: &async_channel::Sender<OpenRequest>,
    kind: NotificationKind,
    reminder: &Reminder,
) -> Result<(), String> {
    match kind {
        NotificationKind::System => {
            notify::show_system_notification(&reminder.title, reminder.body())
        }
        NotificationKind::App => {
            let (reply, response) = async_channel::bounded(1);
            requests
                .send_blocking(OpenRequest {
                    content: NotificationContent::from(reminder),
                    reply,
                })
                .map_err(|_| "应用正在退出".to_string())?;
            response
                .recv_blocking()
                .map_err(|_| "应用正在退出".to_string())?
        }
    }
}

/// 启动提醒调度（`run()` 里调用一次）
pub fn start(cx: &mut App) {
    let (requests, incoming) = async_channel::unbounded::<OpenRequest>();
    // 前台：按请求打开通知窗口
    cx.spawn(async move |cx| {
        while let Ok(request) = incoming.recv().await {
            let opened = cx.update(|cx| notification::open(cx, request.content).is_some());
            let result = if opened {
                Ok(())
            } else {
                Err("创建通知窗口失败".to_string())
            };
            let _ = request.reply.send(result).await;
        }
    })
    .detach();

    let db = Db::get(cx);
    let executor = cx.background_executor().clone();
    cx.background_executor()
        .spawn(async move {
            // 等待应用初始化完成
            executor.timer(reminder::STARTUP_DELAY).await;
            let kind = reminder::notification_kind(&db);
            if let Err(e) =
                reminder::catch_up_missed_repeats(&db, &mut |r| deliver(&requests, kind, r))
            {
                log::error!("[notify] 补发错过的重复提醒失败: {e}");
            }
            loop {
                executor
                    .timer(reminder::duration_until_next_minute(SystemTime::now()))
                    .await;
                // 每轮重新读取通知方式（设置里改了立即生效）
                let kind = reminder::notification_kind(&db);
                if let Err(e) = reminder::run_due(&db, &mut |r| deliver(&requests, kind, r)) {
                    log::error!("[notify] 通知检查失败: {e}");
                }
            }
        })
        .detach();
}
