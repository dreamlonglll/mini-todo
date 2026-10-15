//! 已完成窗口（旧 `CompletedView.vue`）。规格：`research/ui-editor-windows.md` §10。
//!
//! 窗口 460×550、可缩放、不透明，打开方是主窗口（owner + 遮罩 0.3）。从这里点开的编辑窗口以本窗口为打开方
//! （遮罩 0.3，遮罩按下或本窗口获得焦点时把编辑窗口带回前台），关闭后重新加载列表。
//!
//! 数据：`core::todo::list` → 只留已完成 → 按 `normalize(updatedAt || createdAt)` 降序（字符串比较）；只在打开时、
//! 恢复 / 删除后、编辑窗口关闭后重新加载（不轮询）。搜索：标题或描述原文（小写）包含关键字。无快捷键。

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};
use mini_todo_core::db::Todo;

use super::editor::EditorMode;
use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size};
use crate::assets::{AppIcon, EpIcon};
use crate::db::spawn_db;
use crate::state::{AppState, update_request};
use crate::theme::{self, hex, light, w2};
use crate::ui::icon::icon;
use crate::ui::title_bar::drag_region;
use crate::ui::tooltip::EpTooltip;
use crate::ui::{message, message_box};
use crate::util::datetime::{self, DateTimeKind};

pub const WIDTH: f32 = 460.;
pub const HEIGHT: f32 = 550.;

/// 排序键：`normalizeDateTime(updatedAt || createdAt) ?? 原值`
fn sort_key(todo: &Todo) -> String {
    let raw = if todo.updated_at.is_empty() {
        &todo.created_at
    } else {
        &todo.updated_at
    };
    datetime::normalize_date_time(Some(raw), DateTimeKind::Generic).unwrap_or_else(|| raw.clone())
}

/// 已完成的待办，按更新时间降序
pub fn completed_sorted(todos: Vec<Todo>) -> Vec<Todo> {
    let mut list: Vec<Todo> = todos.into_iter().filter(|t| t.completed).collect();
    list.sort_by_key(|t| std::cmp::Reverse(sort_key(t)));
    list
}

/// 搜索：空 → 全部；否则标题或描述（原始 Markdown）小写包含关键字
pub fn matches_query(todo: &Todo, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || todo.title.to_lowercase().contains(&q)
        || todo
            .description
            .as_deref()
            .is_some_and(|d| d.to_lowercase().contains(&q))
}

/// 已完成窗口视图
pub struct CompletedWindow {
    todos: Vec<Todo>,
    search: Entity<InputState>,
    hovered: Option<i64>,
    _subscriptions: Vec<Subscription>,
}

impl CompletedWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索已完成的待办..."));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // 本窗口获得焦点时把模态子窗口（编辑窗口）带回前台
            cx.observe_window_activation(window, |_, window, cx| {
                WindowManager::forward_activation(window, cx);
            }),
            cx.observe_global::<WindowManager>(|_, cx| cx.notify()),
        ];
        let mut this = Self {
            todos: Vec::new(),
            search,
            hovered: None,
            _subscriptions: subscriptions,
        };
        this.load(window, cx);
        this
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = spawn_db(cx, mini_todo_core::todo::list);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(todos) => {
                    this.todos = completed_sorted(todos);
                    cx.notify();
                }
                Err(e) => message::error_with(window, cx, "加载已完成待办失败", &e),
            });
        })
        .detach();
    }

    fn filtered(&self, cx: &App) -> Vec<&Todo> {
        let query = self.search.read(cx).value().to_string();
        self.todos
            .iter()
            .filter(|t| matches_query(t, &query))
            .collect()
    }

    fn restore(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let request = mini_todo_core::db::UpdateTodoRequest {
            completed: Some(false),
            ..update_request()
        };
        let task = AppState::global(cx).update(cx, |state, cx| state.update_todo(id, request, cx));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(_) => this.load(window, cx),
                Err(e) => message::error_with(window, cx, "恢复待办失败", &e),
            });
        })
        .detach();
    }

    fn delete(&mut self, id: i64, title: String, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        message_box::confirm(
            window,
            cx,
            message_box::ConfirmOptions::new("删除确认", format!("确定要删除待办\"{title}\"吗？"))
                .confirm_text("删除")
                .cancel_text("取消")
                .kind(message_box::BoxKind::Warning),
            move |confirmed, window, cx| {
                if !confirmed {
                    return;
                }
                let _ = weak.update(cx, |_, cx| {
                    let task =
                        AppState::global(cx).update(cx, |state, cx| state.delete_todo(id, cx));
                    cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
                        let result = task.await;
                        let _ = this.update_in(cx, |this, window, cx| match result {
                            Ok(()) => this.load(window, cx),
                            Err(e) => message::error_with(window, cx, "删除待办失败", &e),
                        });
                    })
                    .detach();
                });
            },
        );
    }

    /// 点击行：打开编辑窗口（只读详情，模态）；关闭后重新加载
    fn open_editor(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let opener = window.window_handle().window_id();
        if WindowManager::focus_modal_child(cx, opener) {
            return;
        }
        let parent = ChildOf::window(window, cx);
        let Some(handle) = super::editor::open(cx, Some(parent), Some(id), EditorMode::View) else {
            message::error(window, cx, "打开编辑器失败");
            return;
        };
        let weak = cx.entity().downgrade();
        let own = window.window_handle();
        WindowManager::on_closed(cx, handle, move |cx| {
            let _ = own.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.load(window, cx));
            });
        });
        cx.notify();
    }

    fn render_title_bar(&self, count: usize) -> impl IntoElement {
        let palette = light();
        div()
            .relative()
            .flex_none()
            .h(px(42.))
            .pl(px(16.))
            .pr(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .bg(palette.bg_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(drag_region())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(14.))
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .text_color(palette.text_primary)
                    .child("已完成")
                    .child(
                        div()
                            .min_w(px(20.))
                            .h(px(20.))
                            .px(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(10.))
                            .bg(palette.border)
                            .text_size(px(12.))
                            .font_weight(theme::WEIGHT_MEDIUM)
                            .text_color(palette.text_secondary)
                            .child(count.to_string()),
                    ),
            )
            .child(
                div()
                    .id("completed-close")
                    .group("completed-close")
                    .occlude()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_color(palette.text_tertiary)
                    .hover(|s| s.bg(hex(w2::RED_100)))
                    .on_click(|_: &ClickEvent, window, _| window.remove_window())
                    .child(
                        icon(AppIcon::CloseThin, 10., palette.text_tertiary)
                            .group_hover("completed-close", |s| s.text_color(palette.error)),
                    ),
            )
    }

    fn render_search(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let focused = self
            .search
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let has_text = !self.search.read(cx).value().is_empty();
        div().flex_none().px(px(16.)).py(px(12.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .h(px(36.))
                .rounded(px(8.))
                .border_1()
                .when(focused, |this| {
                    this.bg(hex(0xFFFFFF))
                        .border_color(hex(w2::BLUE_300))
                        .shadow(vec![theme::shadow(
                            0.,
                            0.,
                            0.,
                            3.,
                            theme::rgba_css(59, 130, 246, 0.1),
                        )])
                })
                .when(!focused, |this| {
                    this.bg(palette.bg_tertiary)
                        .border_color(gpui_kit::transparent_black())
                })
                .child(icon(EpIcon::Search, 16., palette.text_tertiary).flex_none())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(hex(w2::SLATE_700))
                        .child(
                            Input::new(&self.search)
                                .appearance(false)
                                .h(px(20.))
                                .px(px(0.))
                                .py(px(0.))
                                .text_size(px(13.)),
                        ),
                )
                .when(has_text, |this| {
                    this.child(
                        div()
                            .id("completed-search-clear")
                            .group("completed-search-clear")
                            .flex_none()
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(palette.border))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.search
                                    .update(cx, |state, cx| state.set_value("", window, cx));
                                cx.notify();
                            }))
                            .child(
                                icon(EpIcon::Close, 14., palette.text_tertiary)
                                    .group_hover("completed-search-clear", |s| {
                                        s.text_color(palette.text_secondary)
                                    }),
                            ),
                    )
                }),
        )
    }

    fn render_item(&self, todo: &Todo, last: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let id = todo.id;
        let hovered = self.hovered == Some(id);
        let time = todo
            .notify_at
            .as_deref()
            .and_then(|v| datetime::format_date_time(Some(v), "MM-DD HH:mm", DateTimeKind::Notify));
        let title = todo.title.clone();
        let action = |name: &'static str,
                      source: EpIcon,
                      bg: u32,
                      fg: gpui_kit::Hsla,
                      hover_bg: u32,
                      hover_fg: u32| {
            let group = SharedString::from(format!("completed-{name}-{id}"));
            div()
                .id(group.clone())
                .group(group.clone())
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .bg(hex(bg))
                .text_color(fg)
                .hover(move |s| s.bg(hex(hover_bg)))
                .child(
                    icon(source, 16., fg).group_hover(group, move |s| s.text_color(hex(hover_fg))),
                )
        };
        let restore = action(
            "restore",
            EpIcon::RefreshLeft,
            w2::BLUE_100,
            palette.primary,
            w2::BLUE_200,
            0x2563EB,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.restore(id, window, cx);
        }));
        let delete = action(
            "delete",
            EpIcon::Delete,
            w2::RED_100,
            palette.error,
            w2::RED_200,
            w2::RED_600,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.delete(id, title.clone(), window, cx);
        }));
        div()
            .id(SharedString::from(format!("completed-item-{id}")))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .py(px(12.))
            .when(!last, |this| this.mb(px(8.)))
            .rounded(px(10.))
            .cursor_pointer()
            .bg(if hovered {
                palette.bg_tertiary
            } else {
                palette.bg_secondary
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered = Some(id);
                } else if this.hovered == Some(id) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_editor(id, window, cx)
                }),
            )
            .child(
                div()
                    .flex_none()
                    .size(px(8.))
                    .rounded_full()
                    .bg(theme::todo_color(&todo.color, palette.success)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(palette.text_secondary)
                            .line_through()
                            .truncate()
                            .child(todo.title.clone()),
                    )
                    .when_some(time, |this, time| {
                        this.child(
                            div()
                                .mt(px(4.))
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .text_size(px(12.))
                                .text_color(palette.text_tertiary)
                                .child(icon(EpIcon::Clock, 12., palette.text_tertiary))
                                .child(time),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .opacity(if hovered { 1. } else { 0. })
                    .child(EpTooltip::new(
                        SharedString::from(format!("completed-restore-tip-{id}")),
                        "恢复为未完成",
                        restore,
                    ))
                    .child(EpTooltip::new(
                        SharedString::from(format!("completed-delete-tip-{id}")),
                        "删除",
                        delete,
                    )),
            )
    }
}

impl Render for CompletedWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let filtered: Vec<Todo> = self.filtered(cx).into_iter().cloned().collect();
        let count = filtered.len();
        let searching = !self.search.read(cx).value().trim().is_empty();
        let empty_state = |source: EpIcon, text: &'static str| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .px(px(20.))
                .py(px(60.))
                .text_color(palette.text_tertiary)
                .child(div().mb(px(12.)).opacity(0.5).child(icon(
                    source,
                    40.,
                    palette.text_tertiary,
                )))
                .child(div().text_size(px(14.)).child(text))
        };
        let items: Vec<gpui_kit::AnyElement> = filtered
            .iter()
            .enumerate()
            .map(|(index, todo)| {
                self.render_item(todo, index + 1 == count, cx)
                    .into_any_element()
            })
            .collect();
        let list = div()
            .id("completed-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(16.))
            .pb(px(16.))
            .children(items)
            .when(count == 0 && searching, |this| {
                this.child(empty_state(EpIcon::Search, "未找到匹配的待办"))
            })
            .when(count == 0 && !searching, |this| {
                this.child(empty_state(EpIcon::Check, "暂无已完成的待办"))
            });
        let search = self.render_search(window, cx).into_any_element();
        div()
            .id("completed-window")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(gpui_kit::relative(theme::LINE_HEIGHT))
            .text_color(palette.text_primary)
            .bg(hex(0xFFFFFF))
            .child(self.render_title_bar(count))
            .child(search)
            .child(list)
            .children(super::modal_overlay(window, cx, 0.3))
    }
}

/// 打开已完成窗口（打开方：主窗口 `ChildOf::window(window, cx)`；截图调试 `None`）
pub fn open(cx: &mut App, parent: Option<ChildOf>) -> Option<AnyWindowHandle> {
    let spec = WindowSpec::new(
        "已完成",
        gpui_kit::Bounds::new(Default::default(), logical_size(WIDTH, HEIGHT)),
    );
    WindowManager::open_child(
        cx,
        parent.as_ref(),
        WindowKind::Completed,
        None,
        spec,
        |window, cx| cx.new(|cx| CompletedWindow::new(window, cx)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(id: i64, title: &str, completed: bool, updated: &str, created: &str) -> Todo {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": title,
            "description": if id == 2 { Some("含 Keyword 的描述") } else { None },
            "color": "#10B981",
            "quadrant": 4,
            "notifyAt": null,
            "startTime": null,
            "endTime": null,
            "completed": completed,
            "updatedAt": updated,
            "createdAt": created,
        }))
        .unwrap()
    }

    #[test]
    fn keeps_completed_sorted_by_update_time_desc() {
        let list = completed_sorted(vec![
            todo(1, "a", true, "2026-10-10 08:00:00", "2026-10-01 00:00:00"),
            todo(2, "b", true, "2026-10-12T09:00", "2026-10-01 00:00:00"),
            todo(3, "c", false, "2026-10-13 09:00:00", "2026-10-01 00:00:00"),
            todo(4, "d", true, "", "2026-10-11 00:00:00"),
        ]);
        let ids: Vec<i64> = list.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![2, 4, 1]);
    }

    #[test]
    fn search_matches_title_or_description_case_insensitively() {
        let t = todo(2, "Weekly Report", true, "", "");
        assert!(matches_query(&t, ""));
        assert!(matches_query(&t, "  weekly "));
        assert!(matches_query(&t, "keyword"));
        assert!(!matches_query(&t, "missing"));
    }
}
