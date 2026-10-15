//! 保存逻辑（§3.8）：防重入、新建 / 更新、`buildUpdateRequest`、完成 / 重新打开、暂存子任务落库。
//!
//! 所有写操作走 [`AppState`]（同进程），主窗口的本地副本随之更新；失败提示 `{前缀}：{详情}`。

use gpui_kit::{App, AsyncWindowContext, Context, WeakEntity, Window};
use mini_todo_core::db::{
    CreateSubTaskRequest, CreateTodoRequest, Todo, UpdateSubTaskRequest, UpdateTodoRequest,
};

use super::{EditorWindow, RepeatType};
use crate::state::{AppState, update_request};
use crate::ui::message;
use crate::util::datetime::{self, DateTimeKind};

/// 保存时的表单值（已把自定义提前分钟数合进 `notify_before`）
#[derive(Debug, Clone, PartialEq)]
pub(super) struct FormValues {
    pub title: String,
    /// 存储形式
    pub description: String,
    pub color: Option<String>,
    pub quadrant: i32,
    pub notify_at: Option<String>,
    pub notify_before: i32,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub repeat_enabled: bool,
    pub repeat_type: RepeatType,
    pub repeat_interval: i32,
    pub repeat_weekdays: Vec<u8>,
    pub repeat_month_day: i32,
}

/// 加载时的原值（判断"清除"与"提醒时间没改"）
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Originals {
    pub notify_at: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub repeat_enabled: bool,
}

/// `create_todo` 请求（`color` 为空时返回 `None`：旧版反序列化失败报"创建待办失败"）
pub(super) fn build_create_request(form: &FormValues) -> Option<CreateTodoRequest> {
    Some(CreateTodoRequest {
        title: form.title.clone(),
        description: (!form.description.is_empty()).then(|| form.description.clone()),
        color: form.color.clone()?,
        quadrant: form.quadrant,
        notify_at: form.notify_at.clone(),
        notify_before: Some(if form.repeat_enabled {
            0
        } else {
            form.notify_before
        }),
        start_time: form.start_time.clone(),
        end_time: form.end_time.clone(),
    })
}

/// 旧 `buildUpdateRequest()`
pub(super) fn build_update_request(form: &FormValues, original: &Originals) -> UpdateTodoRequest {
    let should_clear_notify_at = original.notify_at.is_some() && form.notify_at.is_none();
    let should_clear_start = original.start_time.is_some() && form.start_time.is_none();
    let should_clear_end = original.end_time.is_some() && form.end_time.is_none();
    let should_clear_repeat = original.repeat_enabled && !form.repeat_enabled;
    // 提醒时间没改就不发（后端收到 notifyAt 会把 notified 复位，已提醒过的待办改个标题会再弹一次）
    let notify_unchanged =
        datetime::is_same_minute(
            form.notify_at.as_deref(),
            original.notify_at.as_deref(),
            DateTimeKind::Notify,
        ) && datetime::normalize_date_time(original.notify_at.as_deref(), DateTimeKind::Notify)
            == original.notify_at;
    let send_notify_at = form.notify_at.is_some() && (!notify_unchanged || form.repeat_enabled);
    let repeat = form.repeat_enabled;
    let weekdays = {
        let mut days = form.repeat_weekdays.clone();
        days.sort_unstable();
        days.dedup();
        days.iter().map(u8::to_string).collect::<Vec<_>>().join(",")
    };
    UpdateTodoRequest {
        title: Some(form.title.clone()),
        // 清空时也发空串：后端把缺省当"不修改"
        description: Some(form.description.clone()),
        color: form.color.clone(),
        quadrant: Some(form.quadrant),
        notify_at: if send_notify_at {
            form.notify_at.clone()
        } else {
            None
        },
        notify_before: Some(if repeat { 0 } else { form.notify_before }),
        clear_notify_at: should_clear_notify_at,
        start_time: form.start_time.clone(),
        end_time: form.end_time.clone(),
        clear_start_time: should_clear_start,
        clear_end_time: should_clear_end,
        clear_repeat: should_clear_repeat,
        repeat_enabled: repeat.then_some(true),
        repeat_type: repeat.then(|| form.repeat_type.as_str().to_string()),
        repeat_interval: repeat.then_some(form.repeat_interval),
        repeat_weekdays: (repeat && form.repeat_type == RepeatType::Weekly && !weekdays.is_empty())
            .then_some(weekdays),
        repeat_month_day: (repeat && form.repeat_type == RepeatType::Monthly)
            .then_some(form.repeat_month_day),
        ..update_request()
    }
}

impl EditorWindow {
    fn form_values(&self, cx: &App) -> FormValues {
        FormValues {
            title: self.title.read(cx).value().to_string(),
            description: self.description_value(cx),
            color: self.color.clone(),
            quadrant: self.quadrant,
            notify_at: self.form_notify_at(cx),
            notify_before: self.notify_before,
            start_time: self.form_start_time(cx),
            end_time: self.form_end_time(cx),
            repeat_enabled: self.repeat_enabled,
            repeat_type: self.repeat_type,
            repeat_interval: self.repeat_interval_value(cx),
            repeat_weekdays: self.repeat_weekdays.clone(),
            repeat_month_day: self.repeat_month_day_value(cx),
        }
    }

    fn originals(&self) -> Originals {
        Originals {
            notify_at: self.original_notify_at.clone(),
            start_time: self.original_start_time.clone(),
            end_time: self.original_end_time.clone(),
            repeat_enabled: self.todo.as_ref().is_some_and(|t| t.repeat_enabled),
        }
    }

    /// 旧 `handleSave()`（防重入；空标题静默忽略；标题不 trim 原样提交）
    pub(super) fn handle_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.title.read(cx).value().trim().is_empty() {
            return;
        }
        if self.is_custom_notify_before {
            self.notify_before = self.custom_before(cx);
        }
        self.saving = true;
        cx.notify();
        match self.todo_id {
            Some(id) => self.save_existing(id, window, cx),
            None => self.save_new(window, cx),
        }
    }

    fn finish_saving(this: &WeakEntity<Self>, cx: &mut AsyncWindowContext) {
        let _ = this.update(cx, |this, cx| {
            this.saving = false;
            cx.notify();
        });
    }

    /// 旧 `saveNewTodo()`
    fn save_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let form = self.form_values(cx);
        let Some(request) = build_create_request(&form) else {
            self.saving = false;
            message::error_with(window, cx, "创建待办失败", "颜色不能为空");
            cx.notify();
            return;
        };
        let state = AppState::global(cx);
        let task = state.update(cx, |state, cx| state.create_todo(request, cx));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let created = match task.await {
                Ok(todo) => todo,
                Err(e) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        message::error_with(window, cx, "创建待办失败", &e)
                    });
                    Self::finish_saving(&this, cx);
                    return;
                }
            };
            let mut ok = true;
            // create_todo 不接受重复字段：再 update 一次
            if form.repeat_enabled {
                let request = build_update_request(&form, &Originals::default());
                let id = created.id;
                let task = cx.update(|_, cx| {
                    AppState::global(cx).update(cx, |state, cx| state.update_todo(id, request, cx))
                });
                if let Ok(task) = task
                    && let Err(e) = task.await
                {
                    ok = false;
                    let _ = this.update_in(cx, |_, window, cx| {
                        message::error_with(
                            window,
                            cx,
                            "待办已创建，但重复提醒未保存，请点击「保存」重试",
                            &e,
                        )
                    });
                }
            }
            if ok {
                ok = Self::flush_pending(&this, created.id, cx).await;
            }
            if !ok {
                // 窗口留着并变成"编辑待办"，再点保存走更新分支、不会重复建待办
                let _ = this.update_in(cx, |this, window, cx| {
                    this.enter_edit_state(created);
                    this.load_todo(false, window, cx);
                });
                Self::finish_saving(&this, cx);
                return;
            }
            let _ = this.update_in(cx, |_, window, cx| {
                message::success(window, cx, "待办已创建");
                window.remove_window();
            });
        })
        .detach();
    }

    /// 旧 `saveExistingTodo(id)`
    fn save_existing(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let request = build_update_request(&self.form_values(cx), &self.originals());
        let state = AppState::global(cx);
        let task = state.update(cx, |state, cx| state.update_todo(id, request, cx));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            match task.await {
                Ok(updated) => {
                    let _ = this.update(cx, |this, _| this.enter_edit_state(updated));
                }
                Err(e) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        message::error_with(window, cx, "保存待办失败", &e)
                    });
                    Self::finish_saving(&this, cx);
                    return;
                }
            }
            let has_pending = this
                .update(cx, |this, cx| !this.drafts.read(cx).items.is_empty())
                .unwrap_or(false);
            if has_pending && !Self::flush_pending(&this, id, cx).await {
                let _ = this.update_in(cx, |this, window, cx| this.load_todo(false, window, cx));
                Self::finish_saving(&this, cx);
                return;
            }
            let _ = this.update_in(cx, |_, window, cx| {
                message::success(window, cx, "待办已保存");
                window.remove_window();
            });
        })
        .detach();
    }

    /// 旧 `flushPendingSubtasks(parentId)`：按顺序逐个创建，成功一个移除一个，失败即停（剩下的下次保存重试）
    async fn flush_pending(
        this: &WeakEntity<Self>,
        parent_id: i64,
        cx: &mut AsyncWindowContext,
    ) -> bool {
        let Ok(pending) = this.update(cx, |this, cx| this.drafts.read(cx).items.clone()) else {
            return false;
        };
        for draft in pending {
            let request = CreateSubTaskRequest {
                parent_id,
                title: draft.title.clone(),
                content: draft
                    .content
                    .as_deref()
                    .filter(|c| !c.is_empty())
                    .map(crate::util::image_ref::to_storage_markdown),
            };
            let task = cx.update(|_, cx| {
                AppState::global(cx).update(cx, |state, cx| state.create_subtask(request, cx))
            });
            let Ok(task) = task else {
                return false;
            };
            match task.await {
                Ok(created) => {
                    let _ = this.update(cx, |this, cx| {
                        this.drafts.update(cx, |drafts, cx| {
                            drafts.remove(draft.id);
                            cx.notify();
                        })
                    });
                    if draft.completed {
                        let request = UpdateSubTaskRequest {
                            title: None,
                            content: None,
                            completed: Some(true),
                            sort_order: None,
                        };
                        let task = cx.update(|_, cx| {
                            AppState::global(cx).update(cx, |state, cx| {
                                state.update_subtask(created.id, request, cx)
                            })
                        });
                        if let Ok(task) = task
                            && let Err(e) = task.await
                        {
                            log::error!("[editor] 暂存子任务完成态保存失败: {e}");
                        }
                    }
                }
                Err(e) => {
                    let _ = this.update_in(cx, |this, window, cx| {
                        let remaining = this.drafts.read(cx).items.len();
                        message::error_with(
                            window,
                            cx,
                            &format!("{remaining} 个子任务未能创建，请点击「保存」重试"),
                            &e,
                        );
                    });
                    return false;
                }
            }
        }
        true
    }

    /// 旧 `updateTodoCompleted(completed)`：只提交 `completed`，成功后关窗（无提示）
    pub(super) fn update_todo_completed(
        &mut self,
        completed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.todo_id else {
            return;
        };
        if self.updating_complete || self.todo.as_ref().is_some_and(|t| t.completed == completed) {
            return;
        }
        self.updating_complete = true;
        cx.notify();
        let request = UpdateTodoRequest {
            completed: Some(completed),
            ..update_request()
        };
        let task = AppState::global(cx).update(cx, |state, cx| state.update_todo(id, request, cx));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let result: Result<Todo, String> = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.updating_complete = false;
                match result {
                    Ok(_) => window.remove_window(),
                    Err(e) => message::error_with(
                        window,
                        cx,
                        if completed {
                            "标记完成失败"
                        } else {
                            "重新打开失败"
                        },
                        &e,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> FormValues {
        FormValues {
            title: " 标题 ".into(),
            description: String::new(),
            color: Some("#10B981".into()),
            quadrant: 4,
            notify_at: None,
            notify_before: 15,
            start_time: None,
            end_time: None,
            repeat_enabled: false,
            repeat_type: RepeatType::Daily,
            repeat_interval: 1,
            repeat_weekdays: Vec::new(),
            repeat_month_day: 1,
        }
    }

    #[test]
    fn create_request_omits_empty_description_and_zeroes_before_for_repeat() {
        let mut f = form();
        let request = build_create_request(&f).unwrap();
        assert_eq!(request.title, " 标题 ");
        assert_eq!(request.description, None);
        assert_eq!(request.notify_before, Some(15));
        f.repeat_enabled = true;
        f.description = "x".into();
        let request = build_create_request(&f).unwrap();
        assert_eq!(request.description.as_deref(), Some("x"));
        assert_eq!(request.notify_before, Some(0));
        f.color = None;
        assert!(build_create_request(&f).is_none());
    }

    #[test]
    fn unchanged_notify_time_is_not_resent() {
        let mut f = form();
        f.notify_at = Some("2026-10-20 14:30:00".into());
        let original = Originals {
            notify_at: Some("2026-10-20 14:30:00".into()),
            ..Default::default()
        };
        let request = build_update_request(&f, &original);
        assert_eq!(request.notify_at, None);
        assert!(!request.clear_notify_at);
        // 库里不是规范格式：发一次规范化后的值
        let original = Originals {
            notify_at: Some("2026-10-20T14:30".into()),
            ..Default::default()
        };
        let request = build_update_request(&f, &original);
        assert_eq!(request.notify_at.as_deref(), Some("2026-10-20 14:30:00"));
        // 改了就发
        f.notify_at = Some("2026-10-20 15:00:00".into());
        let request = build_update_request(&f, &original);
        assert_eq!(request.notify_at.as_deref(), Some("2026-10-20 15:00:00"));
        // 开着重复总是发
        f.notify_at = Some("2026-10-20 14:30:00".into());
        f.repeat_enabled = true;
        let original = Originals {
            notify_at: Some("2026-10-20 14:30:00".into()),
            ..Default::default()
        };
        let request = build_update_request(&f, &original);
        assert_eq!(request.notify_at.as_deref(), Some("2026-10-20 14:30:00"));
    }

    #[test]
    fn clear_flags_and_description_always_sent() {
        let f = form();
        let original = Originals {
            notify_at: Some("2026-10-20 14:30:00".into()),
            start_time: Some("2026-10-19 00:00:00".into()),
            end_time: None,
            repeat_enabled: true,
        };
        let request = build_update_request(&f, &original);
        assert!(request.clear_notify_at);
        assert!(request.clear_start_time);
        assert!(!request.clear_end_time);
        assert!(request.clear_repeat);
        assert_eq!(request.description.as_deref(), Some(""));
        assert_eq!(request.repeat_enabled, None);
        assert_eq!(request.repeat_type, None);
        assert_eq!(request.notify_before, Some(15));
    }

    #[test]
    fn repeat_fields_follow_type() {
        let mut f = form();
        f.repeat_enabled = true;
        f.repeat_type = RepeatType::Weekly;
        f.repeat_interval = 2;
        f.repeat_weekdays = vec![5, 1, 3];
        let request = build_update_request(&f, &Originals::default());
        assert_eq!(request.repeat_enabled, Some(true));
        assert_eq!(request.repeat_type.as_deref(), Some("weekly"));
        assert_eq!(request.repeat_interval, Some(2));
        assert_eq!(request.repeat_weekdays.as_deref(), Some("1,3,5"));
        assert_eq!(request.repeat_month_day, None);
        assert_eq!(request.notify_before, Some(0));
        // weekly 但一个没选：不发（后端保留旧值）
        f.repeat_weekdays.clear();
        assert_eq!(
            build_update_request(&f, &Originals::default()).repeat_weekdays,
            None
        );
        f.repeat_type = RepeatType::Monthly;
        f.repeat_month_day = 31;
        let request = build_update_request(&f, &Originals::default());
        assert_eq!(request.repeat_month_day, Some(31));
        assert_eq!(request.repeat_weekdays, None);
    }
}
