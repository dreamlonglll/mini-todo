//! 提醒调度（旧 `services/notification.rs::start_scheduler`）。
//!
//! 循环（与旧版一致）：`sleep(STARTUP_DELAY)` → `catch_up_missed_repeats`（补发错过的重复提醒）→ 每到整分钟
//! `notification_kind` + `run_due`。查询、推进状态都在后台线程；Windows 的系统通知也在后台线程发。
//! 要在主线程做的事经通道交给前台并等它回话：
//! - 应用内通知要建窗口：窗口没建成返回 `Err`，core 就不推进这条提醒、下一分钟重试（与旧版 `send_app_notification`
//!   返回 `Err` 时一致）；
//! - macOS 的系统通知（`UNUserNotificationCenter` 只能在主线程调用）：交给前台即视为已投递（拿不到投递结果，见 `notify.rs`）。
//!
//! 单条提醒出错只记日志，不影响同一轮的其它提醒（core `run_due` 负责）。

use std::time::SystemTime;

use gpui_kit::App;
use mini_todo_core::reminder::{self, NotificationKind, Reminder};

use crate::db::Db;
#[cfg(not(target_os = "macos"))]
use crate::services::notify;
use crate::windows::notification::{self, NotificationContent};

/// 要在前台（主线程）完成的投递
enum ForegroundJob {
    /// 打开应用内通知窗口
    OpenWindow(NotificationContent),
    /// macOS 系统通知
    #[cfg(target_os = "macos")]
    SystemNotification {
        tag: String,
        title: String,
        body: String,
    },
}

impl ForegroundJob {
    fn run(self, cx: &mut App) -> Result<(), String> {
        match self {
            ForegroundJob::OpenWindow(content) => {
                if notification::open(cx, content).is_some() {
                    Ok(())
                } else {
                    Err("创建通知窗口失败".to_string())
                }
            }
            #[cfg(target_os = "macos")]
            ForegroundJob::SystemNotification { tag, title, body } => {
                crate::services::notify::show_system_notification(cx, &tag, &title, &body);
                Ok(())
            }
        }
    }
}

/// 交给前台执行的请求
struct ForegroundRequest {
    job: ForegroundJob,
    reply: async_channel::Sender<Result<(), String>>,
}

/// 把一项投递交给前台并阻塞等待结果（后台线程调用）
fn run_in_foreground(
    requests: &async_channel::Sender<ForegroundRequest>,
    job: ForegroundJob,
) -> Result<(), String> {
    let (reply, response) = async_channel::bounded(1);
    requests
        .send_blocking(ForegroundRequest { job, reply })
        .map_err(|_| "应用正在退出".to_string())?;
    response
        .recv_blocking()
        .map_err(|_| "应用正在退出".to_string())?
}

/// 发系统通知（Windows Toast / Linux：后台线程直接发，失败返回 `Err`）
#[cfg(not(target_os = "macos"))]
fn system_notification(
    _requests: &async_channel::Sender<ForegroundRequest>,
    reminder: &Reminder,
) -> Result<(), String> {
    notify::show_system_notification(&reminder.title, reminder.body())
}

/// 发系统通知（macOS：交给前台）。tag 按"待办 + 提醒时间"：同一次提醒重复投递时替换而不是叠两条，
/// 重复提醒的每一次各占一条
#[cfg(target_os = "macos")]
fn system_notification(
    requests: &async_channel::Sender<ForegroundRequest>,
    reminder: &Reminder,
) -> Result<(), String> {
    run_in_foreground(
        requests,
        ForegroundJob::SystemNotification {
            tag: format!(
                "mini-todo-reminder-{}-{}",
                reminder.id,
                reminder.notify_at.as_deref().unwrap_or_default()
            ),
            title: reminder.title.clone(),
            body: reminder.body().to_string(),
        },
    )
}

/// 发出一条提醒（后台线程调用；需要前台的投递阻塞等待前台的结果）
fn deliver(
    requests: &async_channel::Sender<ForegroundRequest>,
    kind: NotificationKind,
    reminder: &Reminder,
) -> Result<(), String> {
    match kind {
        NotificationKind::System => system_notification(requests, reminder),
        NotificationKind::App => run_in_foreground(
            requests,
            ForegroundJob::OpenWindow(NotificationContent::from(reminder)),
        ),
    }
}

/// 启动提醒调度（`run()` 里调用一次）
pub fn start(cx: &mut App) {
    let (requests, incoming) = async_channel::unbounded::<ForegroundRequest>();
    // 前台：打开通知窗口 / 发 macOS 系统通知
    cx.spawn(async move |cx| {
        while let Ok(request) = incoming.recv().await {
            let result = cx.update(|cx| request.job.run(cx));
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
