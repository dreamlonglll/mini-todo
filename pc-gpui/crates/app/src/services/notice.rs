//! 整个应用的提示（不属于某一个窗口的操作结果）显示在哪里（任务 `11-09-multi-list-windows` 第 5 步）：自动同步失败、
//! 加载待办失败（[`AppStateEvent::Message`]）、导入成功、托盘「新增待办清单」失败。以前固定显示在主窗口；拆成多个清单 / 日历窗口后
//! 默认清单窗口可能隐藏着，提示就落空了。
//!
//! - **目标窗口**（[`pick_target`]）：只在看得见的清单 / 日历窗口里挑（显示中、没最小化、没藏在屏幕边缘，`window_mode::is_shown`）——
//!   当前激活的 → 调用方指定的（导入成功：打开设置的那个窗口）→ 最近一次激活过的 → 默认清单窗口 → 其它（清单按 id、日历最后）。
//! - **都看不见时先存着**（最多 [`MAX_PENDING`] 条，同样的提示只留一条、同一主题只留最新的），等下一次有清单 / 日历窗口被激活
//!   （托盘左键唤起、点任务栏、新开窗口……，`window_mode::on_activated`）时显示在那里；激活后稍等一下再挑（托盘唤起会依次激活
//!   好几个窗口，等最后在最上面的那个）。带主题的提示可以撤回：自动同步恢复后撤掉还没显示的「自动同步失败」（[`retract`]）。
//!
//! 标题栏同步按钮、行操作等**窗口自己的**操作结果仍显示在那个窗口里（`ui::message::show` / `show_in`），不经这里。

use std::time::Duration;

use gpui_kit::{AnyWindowHandle, App, Global, SharedString, Subscription};

use crate::services::window_mode;
use crate::state::{AppState, AppStateEvent};
use crate::ui::message::{self, MessageKind};
use crate::windows::WindowManager;
use crate::windows::block::BlockKey;

/// 都看不见时最多存几条
pub const MAX_PENDING: usize = 3;
/// 窗口被激活后等多久再补显示存着的提示
const FLUSH_DELAY: Duration = Duration::from_millis(300);

/// 可以撤回的提示主题
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    /// 自动同步失败（恢复后撤回还没显示的那条）
    AutoSyncFailed,
}

/// 一条提示
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: MessageKind,
    pub text: SharedString,
    pub topic: Option<Topic>,
}

/// 运行时状态（Global）
#[derive(Default)]
struct Notices {
    /// 没有看得见的窗口时存着的提示（旧的在前）
    pending: Vec<Notice>,
    /// 最近一次被激活的清单 / 日历窗口
    last_active: Option<BlockKey>,
    flush_scheduled: bool,
    _events: Option<Subscription>,
}

impl Global for Notices {}

/// 启动时（`lib.rs::run`，`AppState` 建好之后、加载数据之前）：订阅 `AppState` 的整个应用提示（加载待办失败）
pub fn start(cx: &mut App) {
    let state = AppState::global(cx);
    let subscription = cx.subscribe(&state, |_, event: &AppStateEvent, cx| match event {
        AppStateEvent::Message { kind, text } => show(cx, *kind, text.clone()),
    });
    cx.default_global::<Notices>()._events = Some(subscription);
}

/// 显示一条整个应用的提示（目标窗口见模块文档）
pub fn show(cx: &mut App, kind: MessageKind, text: impl Into<SharedString>) {
    post(cx, notice(kind, text, None), None);
}

/// 同 [`show`]，但当前激活的不是清单 / 日历窗口时优先显示在 `from`（发起操作的窗口：导入成功 = 打开设置的那个窗口）
pub fn show_from(cx: &mut App, from: BlockKey, kind: MessageKind, text: impl Into<SharedString>) {
    post(cx, notice(kind, text, None), Some(from));
}

/// 带主题的提示：还没显示出来时可以 [`retract`]；同一主题存着的只留最新的一条
pub fn show_topic(cx: &mut App, topic: Topic, kind: MessageKind, text: impl Into<SharedString>) {
    post(cx, notice(kind, text, Some(topic)), None);
}

/// 撤回还没显示出来的 `topic` 提示（已经显示的不管，3 秒后自己消失）
pub fn retract(cx: &mut App, topic: Topic) {
    retract_topic(&mut cx.default_global::<Notices>().pending, topic);
}

/// 清单 / 日历窗口被激活（`window_mode::on_activated`）：记下它；有存着的提示时稍等一下补显示
pub fn on_block_activated(cx: &mut App, key: BlockKey) {
    let notices = cx.default_global::<Notices>();
    notices.last_active = Some(key);
    if notices.pending.is_empty() || notices.flush_scheduled {
        return;
    }
    notices.flush_scheduled = true;
    cx.spawn(async move |cx| {
        cx.background_executor().timer(FLUSH_DELAY).await;
        cx.update(flush);
    })
    .detach();
}

fn notice(kind: MessageKind, text: impl Into<SharedString>, topic: Option<Topic>) -> Notice {
    Notice {
        kind,
        text: text.into(),
        topic,
    }
}

fn post(cx: &mut App, notice: Notice, preferred: Option<BlockKey>) {
    match target(cx, preferred) {
        // 延后：调用方可能正在别的窗口（设置窗口）的事件处理里
        Some(handle) => cx.defer(move |cx| message::show_in(handle, cx, notice.kind, notice.text)),
        None => {
            log::info!(
                "[notice] 没有看得见的清单 / 日历窗口，提示先存着：{}",
                notice.text
            );
            enqueue(&mut cx.default_global::<Notices>().pending, notice);
        }
    }
}

/// 补显示存着的提示；仍然没有看得见的窗口就继续存着，等下一次激活
fn flush(cx: &mut App) {
    cx.default_global::<Notices>().flush_scheduled = false;
    let Some(handle) = target(cx, None) else {
        return;
    };
    let pending = std::mem::take(&mut cx.global_mut::<Notices>().pending);
    for notice in pending {
        message::show_in(handle, cx, notice.kind, notice.text);
    }
}

/// 现在该显示在哪个窗口（[`pick_target`]）
fn target(cx: &App, preferred: Option<BlockKey>) -> Option<AnyWindowHandle> {
    let candidates: Vec<Candidate> = window_mode::registered(cx)
        .into_iter()
        .map(|key| Candidate {
            key,
            shown: window_mode::is_shown(cx, key),
        })
        .collect();
    let active = cx
        .active_window()
        .and_then(|handle| WindowManager::block_key_of(cx, handle.window_id()));
    let last_active = cx.try_global::<Notices>().and_then(|n| n.last_active);
    let key = pick_target(&candidates, active, preferred, last_active)?;
    WindowManager::block_window(cx, key)
}

// ----------------------------------------------------------------------
// 纯逻辑
// ----------------------------------------------------------------------

/// 一个开着的清单 / 日历窗口
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub key: BlockKey,
    /// 看得见（显示中、没最小化、没藏在屏幕边缘）
    pub shown: bool,
}

/// 提示显示在哪个窗口：只在看得见的窗口里挑——`active`（当前激活的）→ `preferred`（调用方指定的）→ `last_active`（最近一次激活过的）
/// → 默认清单窗口 → 其它（按 key：清单按 id、日历最后）。都看不见时 `None`
pub fn pick_target(
    candidates: &[Candidate],
    active: Option<BlockKey>,
    preferred: Option<BlockKey>,
    last_active: Option<BlockKey>,
) -> Option<BlockKey> {
    let shown = |key: Option<BlockKey>| {
        key.filter(|key| candidates.iter().any(|c| c.key == *key && c.shown))
    };
    shown(active)
        .or_else(|| shown(preferred))
        .or_else(|| shown(last_active))
        .or_else(|| shown(Some(BlockKey::DEFAULT_LIST)))
        .or_else(|| candidates.iter().filter(|c| c.shown).map(|c| c.key).min())
}

/// 存一条提示：同样的提示（类型 + 文字）只留一条，同一主题只留最新的，最多 [`MAX_PENDING`] 条（丢最旧的）
pub fn enqueue(queue: &mut Vec<Notice>, notice: Notice) {
    queue.retain(|n| {
        let same_topic = notice.topic.is_some() && n.topic == notice.topic;
        let same_text = n.kind == notice.kind && n.text == notice.text;
        !same_topic && !same_text
    });
    queue.push(notice);
    if queue.len() > MAX_PENDING {
        let excess = queue.len() - MAX_PENDING;
        queue.drain(..excess);
    }
}

/// 撤掉 `topic` 的提示
pub fn retract_topic(queue: &mut Vec<Notice>, topic: Topic) {
    queue.retain(|n| n.topic != Some(topic));
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: BlockKey = BlockKey::List(1_762_000_000_000_001);
    const HOME: BlockKey = BlockKey::List(1_762_000_000_000_002);

    fn candidates(shown: &[(BlockKey, bool)]) -> Vec<Candidate> {
        shown
            .iter()
            .map(|&(key, shown)| Candidate { key, shown })
            .collect()
    }

    #[test]
    fn the_active_visible_window_wins() {
        let all = candidates(&[
            (BlockKey::DEFAULT_LIST, true),
            (WORK, true),
            (BlockKey::Calendar, true),
        ]);
        assert_eq!(
            pick_target(&all, Some(BlockKey::Calendar), Some(WORK), Some(WORK)),
            Some(BlockKey::Calendar)
        );
        // 激活的不是清单 / 日历窗口（设置窗口、别的应用）：调用方指定的 → 最近激活的 → 默认清单
        assert_eq!(pick_target(&all, None, Some(WORK), None), Some(WORK));
        assert_eq!(
            pick_target(&all, None, None, Some(BlockKey::Calendar)),
            Some(BlockKey::Calendar)
        );
        assert_eq!(
            pick_target(&all, None, None, None),
            Some(BlockKey::DEFAULT_LIST)
        );
    }

    #[test]
    fn hidden_windows_are_skipped() {
        // 默认清单窗口隐藏着（以前提示就落在这里，看不到）
        let all = candidates(&[
            (BlockKey::DEFAULT_LIST, false),
            (WORK, false),
            (HOME, true),
            (BlockKey::Calendar, true),
        ]);
        assert_eq!(
            pick_target(&all, Some(WORK), Some(BlockKey::DEFAULT_LIST), Some(WORK)),
            Some(HOME),
            "激活的 / 指定的 / 最近激活的都看不见：按 key 取第一个看得见的"
        );
        let only_calendar =
            candidates(&[(BlockKey::DEFAULT_LIST, false), (BlockKey::Calendar, true)]);
        assert_eq!(
            pick_target(&only_calendar, None, None, None),
            Some(BlockKey::Calendar)
        );
        // 没登记的窗口（已关掉 / 没开过）不算
        assert_eq!(
            pick_target(&only_calendar, Some(WORK), Some(WORK), Some(WORK)),
            Some(BlockKey::Calendar)
        );
        let none = candidates(&[(BlockKey::DEFAULT_LIST, false), (BlockKey::Calendar, false)]);
        assert_eq!(pick_target(&none, None, None, None), None);
        assert_eq!(pick_target(&[], None, None, None), None);
    }

    fn n(kind: MessageKind, text: &str, topic: Option<Topic>) -> Notice {
        Notice {
            kind,
            text: text.to_string().into(),
            topic,
        }
    }

    #[test]
    fn pending_notices_are_deduplicated_and_capped() {
        let mut queue = Vec::new();
        enqueue(&mut queue, n(MessageKind::Error, "加载待办失败：a", None));
        enqueue(&mut queue, n(MessageKind::Error, "加载待办失败：a", None));
        assert_eq!(queue.len(), 1, "同样的提示只留一条");
        // 同一主题只留最新的
        enqueue(
            &mut queue,
            n(
                MessageKind::Error,
                "自动同步失败：x",
                Some(Topic::AutoSyncFailed),
            ),
        );
        enqueue(
            &mut queue,
            n(
                MessageKind::Error,
                "自动同步失败：y",
                Some(Topic::AutoSyncFailed),
            ),
        );
        assert_eq!(
            queue.iter().map(|n| n.text.as_ref()).collect::<Vec<_>>(),
            vec!["加载待办失败：a", "自动同步失败：y"]
        );
        // 最多 MAX_PENDING 条，丢最旧的
        enqueue(&mut queue, n(MessageKind::Success, "数据导入成功", None));
        enqueue(&mut queue, n(MessageKind::Error, "新建清单失败：z", None));
        assert_eq!(queue.len(), MAX_PENDING);
        assert_eq!(queue[0].text.as_ref(), "自动同步失败：y");
        assert_eq!(queue[MAX_PENDING - 1].text.as_ref(), "新建清单失败：z");
        // 自动同步恢复：撤掉还没显示的那条
        retract_topic(&mut queue, Topic::AutoSyncFailed);
        assert_eq!(
            queue.iter().map(|n| n.text.as_ref()).collect::<Vec<_>>(),
            vec!["数据导入成功", "新建清单失败：z"]
        );
    }
}
