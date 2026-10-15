//! 右侧子任务面板（§3.11）、从文件导入（§3.12）。
//!
//! 列表来源：已存在的待办 → `todo.subtasks`（直接落库，操作后只刷新当前待办）；新建 → 内存里的
//! [`DraftSubtasks`](super::DraftSubtasks)。拖拽排序只在手柄上开始：拖动中按光标所在行实时让位（SortableJS 的
//! 交换语义），原位置占位 50% 透明，跟随鼠标的拖影 80% 透明；松开（包括拖到窗口外松开）时提交顺序。

use std::future::Future;
use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, DragMoveEvent, Entity,
    Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, canvas, div, linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _,
    px, relative,
};
use mini_todo_core::db::{CreateSubTaskRequest, UpdateSubTaskRequest};

use super::EditorWindow;
use crate::assets::EpIcon;
use crate::db::spawn_db;
use crate::state::AppState;
use crate::theme::{self, editor_theme, hex, light};
use crate::ui::ep_dropdown::{EpDropdownItem, EpDropdownMenu};
use crate::ui::ep_popper::{self, Placement, PopperOptions};
use crate::ui::icon::icon;
use crate::ui::title_bar::drag_region;
use crate::ui::tooltip::native;
use crate::ui::{message, message_box};
use crate::windows::subtask::SubtaskMode;

/// 一行子任务（显示用）
#[derive(Debug, Clone, PartialEq)]
struct Row {
    id: i64,
    title: String,
    completed: bool,
}

/// 行内改名状态
pub(super) struct Renaming {
    id: i64,
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// 拖拽中的子任务
#[derive(Clone)]
struct DraggedSubtask {
    row: Row,
    /// 拖影相对手柄的偏移（拖影要和整行对齐）
    shift: Point<Pixels>,
    width: f32,
}

/// 跟随鼠标的拖影（整行外观，80% 不透明）
struct SubtaskGhost {
    drag: DraggedSubtask,
}

impl Render for SubtaskGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let row = &self.drag.row;
        // 拖影视图的根元素由 GPUI 放在"光标 − 按下点相对手柄的偏移"处（即手柄左上角）；根元素上的相对偏移不生效，
        // 所以在内层用相对定位把整行挪回与原行对齐
        div().child(
            div()
                .relative()
                .left(-self.drag.shift.x)
                .top(-self.drag.shift.y)
                .w(px(self.drag.width))
                .opacity(0.8)
                .font(theme::ui_font())
                .child(
                    row_frame(row.completed)
                        .child(handle_icon())
                        .child(checkbox_visual(row.completed))
                        .child(title_visual(&row.title, row.completed)),
                ),
        )
    }
}

/// 行外框（不含交互）
fn row_frame(completed: bool) -> gpui_kit::Div {
    let palette = light();
    div()
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(10.))
        .bg(if completed {
            palette.bg_secondary
        } else {
            hex(0xFFFFFF)
        })
        .border_1()
        .border_color(palette.border)
        .rounded(px(8.))
}

fn handle_icon() -> impl IntoElement {
    div()
        .flex_none()
        .child(icon(EpIcon::Rank, 14., light().text_tertiary))
}

/// 圆形复选框外观（20×20；勾选：蓝→青渐变 + 白勾）
fn checkbox_visual(checked: bool) -> gpui_kit::Div {
    div()
        .flex_none()
        .size(px(20.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .border_2()
        .when(!checked, |this| {
            this.border_color(hex(editor_theme::SLATE_300))
        })
        .when(checked, |this| {
            this.border_color(gpui_kit::transparent_black())
                .bg(linear_gradient(
                    135.,
                    linear_color_stop(light().primary, 0.),
                    linear_color_stop(hex(editor_theme::CYAN_500), 1.),
                ))
                .child(icon(EpIcon::Check, 12., hex(0xFFFFFF)))
        })
}

fn title_visual(title: &str, completed: bool) -> gpui_kit::Div {
    let palette = light();
    div()
        .flex_1()
        .min_w_0()
        .px(px(4.))
        .py(px(2.))
        .rounded(px(4.))
        .text_size(px(13.))
        .line_height(relative(1.4))
        .text_color(if completed {
            palette.text_tertiary
        } else {
            hex(editor_theme::SLATE_700)
        })
        .when(completed, |this| this.line_through())
        .child(title.to_string())
}

/// 系统文件对话框（取消返回 `None`）
fn pick_import_paths(
    folder: bool,
    cx: &mut App,
) -> impl Future<Output = Option<Vec<PathBuf>>> + use<> {
    #[cfg(windows)]
    {
        let _ = cx;
        async move {
            if folder {
                rfd::AsyncFileDialog::new()
                    .set_title("选择文件夹（递归导入 .md/.txt 文件）")
                    .pick_folder()
                    .await
                    .map(|handle| vec![handle.path().to_path_buf()])
            } else {
                rfd::AsyncFileDialog::new()
                    .set_title("导入子任务（选择 .md/.txt 文件或文件夹）")
                    .add_filter("文本文件", &["md", "txt"])
                    .pick_files()
                    .await
                    .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
            }
        }
    }
    #[cfg(not(windows))]
    {
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: !folder,
            directories: folder,
            multiple: !folder,
            prompt: None,
        });
        async move { receiver.await.ok().and_then(Result::ok).flatten() }
    }
}

impl EditorWindow {
    /// 当前列表（拖拽中按拖拽顺序）
    fn rows(&self, cx: &App) -> Vec<Row> {
        let mut rows: Vec<Row> = if self.is_edit() {
            self.todo
                .as_ref()
                .map(|todo| {
                    todo.subtasks
                        .iter()
                        .map(|s| Row {
                            id: s.id,
                            title: s.title.clone(),
                            completed: s.completed,
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            self.drafts
                .read(cx)
                .items
                .iter()
                .map(|s| Row {
                    id: s.id,
                    title: s.title.clone(),
                    completed: s.completed,
                })
                .collect()
        };
        if let Some(order) = &self.drag_order {
            rows.sort_by_key(|row| {
                order
                    .iter()
                    .position(|id| *id == row.id)
                    .unwrap_or(usize::MAX)
            });
        }
        rows
    }

    // ------------------------------------------------------------------
    // 操作
    // ------------------------------------------------------------------

    /// 添加（输入框回车 / 「添加」按钮）
    pub(super) fn add_subtask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.add_input.read(cx).value().trim().to_string();
        if title.is_empty() {
            return;
        }
        let Some(parent_id) = self.todo_id else {
            self.drafts.update(cx, |drafts, cx| {
                drafts.add(title);
                cx.notify();
            });
            self.add_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            return;
        };
        let request = CreateSubTaskRequest {
            parent_id,
            title,
            content: None,
        };
        let task = AppState::global(cx).update(cx, |state, cx| state.create_subtask(request, cx));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(_) => {
                    this.load_todo(false, window, cx);
                    this.add_input
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
                Err(e) => message::error_with(window, cx, "添加子任务失败", &e),
            });
        })
        .detach();
    }

    fn toggle_subtask(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_edit() {
            self.drafts.update(cx, |drafts, cx| {
                drafts.toggle(id);
                cx.notify();
            });
            return;
        }
        let Some(current) = self
            .todo
            .as_ref()
            .and_then(|t| t.subtasks.iter().find(|s| s.id == id))
            .map(|s| s.completed)
        else {
            return;
        };
        let request = UpdateSubTaskRequest {
            title: None,
            content: None,
            completed: Some(!current),
            sort_order: None,
        };
        self.run_subtask_write(
            AppState::global(cx).update(cx, |state, cx| state.update_subtask(id, request, cx)),
            "更新子任务失败",
            window,
            cx,
        );
    }

    fn delete_subtask(
        &mut self,
        id: i64,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        message_box::confirm(
            window,
            cx,
            message_box::ConfirmOptions::new("删除确认", format!("确定删除子任务\"{title}\"吗？"))
                .confirm_text("删除")
                .cancel_text("取消")
                .kind(message_box::BoxKind::Warning),
            move |confirmed, window, cx| {
                if !confirmed {
                    return;
                }
                let _ = weak.update(cx, |this, cx| {
                    if !this.is_edit() {
                        this.drafts.update(cx, |drafts, cx| {
                            drafts.remove(id);
                            cx.notify();
                        });
                        return;
                    }
                    let task =
                        AppState::global(cx).update(cx, |state, cx| state.delete_subtask(id, cx));
                    this.run_subtask_write(task, "删除子任务失败", window, cx);
                });
            },
        );
    }

    /// 等写操作完成：成功只刷新当前待办，失败提示
    fn run_subtask_write<T: 'static>(
        &mut self,
        task: gpui_kit::Task<Result<T, String>>,
        error_prefix: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(_) => this.load_todo(false, window, cx),
                Err(e) => message::error_with(window, cx, error_prefix, &e),
            });
        })
        .detach();
    }

    /// 双击标题：行内改名（自动聚焦并全选；Enter / 失焦提交，Esc 取消）
    fn start_rename(
        &mut self,
        id: i64,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |state, cx| {
            state.set_value(title, window, cx);
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: false, ..
                }
                | InputEvent::Blur => this.commit_rename(id, window, cx),
                _ => {}
            },
        );
        self.renaming = Some(Renaming {
            id,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn commit_rename(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take().filter(|r| r.id == id) else {
            return;
        };
        cx.notify();
        let title = renaming.input.read(cx).value().trim().to_string();
        if title.is_empty() {
            return;
        }
        let request = UpdateSubTaskRequest {
            title: Some(title),
            content: None,
            completed: None,
            sort_order: None,
        };
        let task =
            AppState::global(cx).update(cx, |state, cx| state.update_subtask(id, request, cx));
        self.run_subtask_write(task, "修改子任务标题失败", window, cx);
        // 焦点还给窗口（快捷键继续可用）
        window.focus(&self.focus, cx);
    }

    /// 拖拽结束：提交当前顺序
    pub(super) fn finish_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(order) = self.drag_order.take() else {
            return;
        };
        self.dragging = None;
        cx.notify();
        let original: Vec<i64> = self.rows(cx).iter().map(|r| r.id).collect();
        if order == original {
            return;
        }
        if !self.is_edit() {
            self.drafts.update(cx, |drafts, cx| {
                drafts.reorder(&order);
                cx.notify();
            });
            return;
        }
        // 本地立即生效，失败再回滚到服务端顺序
        if let Some(todo) = &mut self.todo {
            todo.subtasks.sort_by_key(|s| {
                order
                    .iter()
                    .position(|id| *id == s.id)
                    .unwrap_or(usize::MAX)
            });
        }
        let task = AppState::global(cx).update(cx, |state, cx| state.reorder_subtasks(order, cx));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            if let Err(e) = task.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    message::error_with(window, cx, "子任务排序保存失败", &e);
                    this.load_todo(false, window, cx);
                });
            }
        })
        .detach();
    }

    /// 拖到某一行上：按 SortableJS 的交换语义让位（越过该行中线才换）
    fn drag_over_row(
        &mut self,
        target: i64,
        event: &DragMoveEvent<DraggedSubtask>,
        cx: &mut Context<Self>,
    ) {
        let dragged = event.drag(cx).row.id;
        if dragged == target || !event.bounds.contains(&event.event.position) {
            return;
        }
        let Some(order) = self.drag_order.as_mut() else {
            return;
        };
        let (Some(from), Some(to)) = (
            order.iter().position(|id| *id == dragged),
            order.iter().position(|id| *id == target),
        ) else {
            return;
        };
        let mid = f32::from(event.bounds.origin.y) + f32::from(event.bounds.size.height) / 2.;
        let y = f32::from(event.event.position.y);
        if (from < to && y < mid) || (from > to && y > mid) {
            return;
        }
        let id = order.remove(from);
        order.insert(to, id);
        cx.notify();
    }

    fn import(&mut self, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.import_menu_open = false;
        cx.notify();
        let Some(parent_id) = self.todo_id else {
            message::warning(window, cx, "请先保存待办后再导入子任务");
            return;
        };
        let pick = pick_import_paths(folder, cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let Some(paths) = pick.await.filter(|p| !p.is_empty()) else {
                return;
            };
            let paths: Vec<String> = paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let Ok(task) = cx.update(|_, cx| {
                spawn_db(cx, move |db| {
                    mini_todo_core::todo::import_subtasks_from_paths(db, parent_id, &paths)
                })
            }) else {
                return;
            };
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(created) => {
                    this.load_todo(false, window, cx);
                    message::success(window, cx, format!("成功导入 {} 个子任务", created.len()));
                }
                Err(e) => message::error_with(window, cx, "导入失败", &e),
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // 渲染
    // ------------------------------------------------------------------

    pub(super) fn render_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = light();
        let rows = self.rows(cx);
        let total = rows.len();
        let done = rows.iter().filter(|r| r.completed).count();
        let pending = if self.is_edit() {
            self.drafts.read(cx).items.len()
        } else {
            0
        };
        let header = div()
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.))
            .py(px(12.))
            .min_h(px(57.))
            .border_b_1()
            .border_color(palette.border)
            .bg(hex(0xFFFFFF))
            .child(drag_region())
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .line_height(relative(1.2))
                    .text_color(hex(editor_theme::SLATE_700))
                    .child("子任务"),
            );

        let content = div()
            .flex_1()
            .min_h_0()
            .p(px(16.))
            .flex()
            .flex_col()
            .when(total > 0, |this| this.child(progress_card(done, total)))
            .when(!self.is_view_mode, |this| {
                this.child(self.render_add_box(window, cx))
            })
            .when(pending > 0, |this| {
                this.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .mb(px(12.))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(8.))
                        .bg(hex(editor_theme::AMBER_50))
                        .text_color(hex(editor_theme::AMBER_700))
                        .text_size(px(12.))
                        .child(icon(EpIcon::Warning, 14., hex(editor_theme::AMBER_700)))
                        .child(format!("{pending} 个子任务尚未创建，点击「保存」重试")),
                )
            })
            .child(if total > 0 {
                self.render_list(rows, window, cx).into_any_element()
            } else {
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .px(px(16.))
                    .py(px(24.))
                    .text_color(palette.text_tertiary)
                    .child(div().mb(px(8.)).opacity(0.5).child(icon(
                        EpIcon::List,
                        32.,
                        palette.text_tertiary,
                    )))
                    .child(div().text_size(px(13.)).child("暂无子任务"))
                    .into_any_element()
            });

        div()
            .w(px(super::PANEL_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(hex(editor_theme::PANEL_BG))
            .border_l_1()
            .border_color(palette.border)
            .child(header)
            .child(content)
    }

    /// 添加区：虚线框（聚焦时实线蓝框 + 3px 光晕）
    fn render_add_box(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let focused = self
            .add_input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let has_text = !self.add_input.read(cx).value().trim().is_empty();
        let import = self.is_edit().then(|| {
            let anchor = self.import_anchor.clone();
            div()
                .id("editor-import")
                .group("editor-import")
                .relative()
                .flex_none()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .border_1()
                .border_color(if self.import_menu_open {
                    palette.primary
                } else {
                    hex(editor_theme::SLATE_300)
                })
                .when(self.import_menu_open, |this| {
                    this.bg(hex(editor_theme::BLUE_50))
                })
                .cursor_pointer()
                .hover(|s| {
                    s.border_color(palette.primary)
                        .bg(hex(editor_theme::BLUE_50))
                })
                .tooltip(native("导入子任务"))
                .child(anchor.measure())
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation()
                })
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.import_menu_open = !this.import_menu_open;
                    cx.notify();
                }))
                .child(
                    icon(
                        EpIcon::Upload,
                        14.,
                        if self.import_menu_open {
                            palette.primary
                        } else {
                            palette.text_secondary
                        },
                    )
                    .group_hover("editor-import", |s| s.text_color(palette.primary)),
                )
        });
        div()
            .id("editor-add-box")
            .flex_none()
            .mb(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(8.))
            .border_1()
            .when(focused, |this| {
                this.bg(hex(0xFFFFFF))
                    .border_color(palette.primary)
                    .shadow(vec![theme::shadow(
                        0.,
                        0.,
                        0.,
                        3.,
                        theme::rgba_css(59, 130, 246, 0.1),
                    )])
            })
            .when(!focused, |this| {
                this.bg(palette.bg_secondary)
                    .border_color(hex(editor_theme::SLATE_300))
                    .border_dashed()
                    .hover(|s| {
                        s.border_color(palette.text_tertiary)
                            .bg(palette.bg_tertiary)
                    })
            })
            .child(icon(EpIcon::Plus, 16., palette.text_tertiary).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .text_color(hex(editor_theme::SLATE_700))
                    .child(
                        Input::new(&self.add_input)
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
                        .id("editor-add-btn")
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(12.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .bg(palette.primary)
                        .hover(|s| s.bg(palette.primary_dark))
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(hex(0xFFFFFF))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.add_subtask(window, cx)
                        }))
                        .child(icon(EpIcon::Plus, 12., hex(0xFFFFFF)))
                        .child("添加"),
                )
            })
            .children(import)
    }

    /// 导入菜单（`el-dropdown`，挂在导入按钮下方、带箭头）
    pub(super) fn render_import_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.import_menu_open || !self.is_edit() || self.is_view_mode {
            return None;
        }
        let dismiss = cx.entity().downgrade();
        let menu = EpDropdownMenu::new("editor-import-menu")
            .item(EpDropdownItem::new("file", "选择文件 (.md/.txt)"))
            .item(EpDropdownItem::new("folder", "选择文件夹（递归导入）"))
            .on_select(cx.listener(|this, command: &SharedString, window, cx| {
                this.import(command.as_ref() == "folder", window, cx)
            }));
        ep_popper::popper(
            &self.import_anchor,
            window,
            PopperOptions {
                placement: Placement::Bottom,
                flip: true,
                gap: ep_popper::OFFSET,
                arrow: true,
                width: 186.,
                height: EpDropdownMenu::estimated_height(2, 0),
            },
            menu,
            move |_, cx| {
                let _ = dismiss.update(cx, |this, cx| {
                    this.import_menu_open = false;
                    cx.notify();
                });
            },
        )
    }

    fn render_list(
        &mut self,
        rows: Vec<Row>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = rows.len();
        let children: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                self.render_row(row, index + 1 == count, window, cx)
                    .into_any_element()
            })
            .collect();
        div()
            .id("editor-subtask-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.panel_scroll)
            .pr(px(4.))
            .children(children)
    }

    fn render_row(
        &mut self,
        row: Row,
        last: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = light();
        let id = row.id;
        let hovered = self.hovered_row == Some(id) && self.drag_order.is_none();
        let is_placeholder = self.dragging == Some(id);
        let can_edit = !self.is_view_mode;
        let can_rename = self.is_edit() && !self.is_view_mode;
        let renaming = self
            .renaming
            .as_ref()
            .filter(|r| r.id == id)
            .map(|r| r.input.clone());
        let bounds_cell = self.row_bounds.clone();

        // 手柄：只有它能开始拖拽
        let drag_value = {
            let height = self
                .row_bounds
                .borrow()
                .get(&id)
                .map_or(44., |b| f32::from(b.size.height));
            let width = self
                .row_bounds
                .borrow()
                .get(&id)
                .map_or(343., |b| f32::from(b.size.width));
            DraggedSubtask {
                row: row.clone(),
                shift: point(px(13.), px((height - 14.) / 2.)),
                width,
            }
        };
        let weak = cx.entity().downgrade();
        let handle = div()
            .id(SharedString::from(format!("subtask-handle-{id}")))
            .flex_none()
            .cursor_grab()
            .opacity(if hovered { 1. } else { 0. })
            .tooltip(native("拖拽排序"))
            .on_drag(drag_value, move |drag, _, _, cx| {
                let drag = drag.clone();
                let _ = weak.update(cx, |this, cx| {
                    this.drag_order = Some(this.rows(cx).iter().map(|r| r.id).collect());
                    this.dragging = Some(drag.row.id);
                    this.hovered_row = None;
                    cx.notify();
                });
                cx.new(|_| SubtaskGhost { drag })
            })
            .child(icon(EpIcon::Rank, 14., palette.text_tertiary));

        let checkbox = checkbox_visual(row.completed)
            .id(SharedString::from(format!("subtask-check-{id}")))
            .cursor_pointer()
            .when(!row.completed, |this| {
                this.hover(|s| s.border_color(palette.primary))
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.toggle_subtask(id, window, cx)
            }));

        let title: AnyElement = match renaming {
            Some(input) => div()
                .flex_1()
                .min_w_0()
                .px(px(4.))
                .py(px(1.))
                .rounded(px(4.))
                .border_1()
                .border_color(palette.primary)
                .bg(hex(0xFFFFFF))
                .text_size(px(13.))
                .text_color(hex(editor_theme::SLATE_700))
                .child(
                    Input::new(&input)
                        .appearance(false)
                        .h(px(18.))
                        .px(px(0.))
                        .py(px(0.))
                        .text_size(px(13.)),
                )
                .into_any_element(),
            None => {
                let title_text = row.title.clone();
                title_visual(&row.title, row.completed)
                    .id(SharedString::from(format!("subtask-title-{id}")))
                    .hover(|s| s.bg(palette.bg_tertiary))
                    .when(can_rename, |this| {
                        this.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            if event.click_count() == 2 {
                                this.start_rename(id, title_text.clone(), window, cx);
                            }
                        }))
                    })
                    .into_any_element()
            }
        };

        let actions = (hovered && self.renaming.as_ref().is_none_or(|r| r.id != id)).then(|| {
            let action = |name: &'static str,
                          source: EpIcon,
                          tip: &'static str,
                          hover_bg: u32,
                          hover_fg: u32| {
                let group = SharedString::from(format!("subtask-{name}-{id}"));
                div()
                    .id(group.clone())
                    .group(group.clone())
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(palette.text_tertiary)
                    .hover(move |s| s.bg(hex(hover_bg)))
                    .tooltip(native(tip))
                    .child(
                        icon(source, 14., palette.text_tertiary)
                            .group_hover(group, move |s| s.text_color(hex(hover_fg))),
                    )
            };
            let title = row.title.clone();
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(6.))
                .flex()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .px(px(4.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(theme::rgba_css(255, 255, 255, 0.95))
                        .shadow(vec![theme::shadow(
                            0.,
                            1.,
                            4.,
                            0.,
                            theme::rgba_css(0, 0, 0, 0.1),
                        )])
                        .child(
                            action(
                                "view",
                                EpIcon::View,
                                "查看子任务",
                                editor_theme::INDIGO_100,
                                editor_theme::INDIGO_500,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, window, cx| {
                                    this.open_subtask_window(id, SubtaskMode::View, window, cx)
                                },
                            )),
                        )
                        .when(can_edit, |this| {
                            this.child(
                                action(
                                    "edit",
                                    EpIcon::Edit,
                                    "编辑子任务",
                                    editor_theme::BLUE_100,
                                    0x3B82F6,
                                )
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, window, cx| {
                                        this.open_subtask_window(id, SubtaskMode::Edit, window, cx)
                                    },
                                )),
                            )
                            .child(
                                action(
                                    "delete",
                                    EpIcon::Delete,
                                    "删除子任务",
                                    editor_theme::RED_100,
                                    0xEF4444,
                                )
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, window, cx| {
                                        this.delete_subtask(id, title.clone(), window, cx)
                                    },
                                )),
                            )
                        }),
                )
        });

        row_frame(row.completed)
            .id(SharedString::from(format!("subtask-row-{id}")))
            .when(!last, |this| this.mb(px(6.)))
            .when(hovered, |this| {
                this.border_color(hex(editor_theme::SLATE_300))
                    .shadow(vec![theme::shadow(
                        0.,
                        2.,
                        4.,
                        0.,
                        theme::rgba_css(0, 0, 0, 0.04),
                    )])
            })
            .when(is_placeholder, |this| this.opacity(0.5))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_row = Some(id);
                } else if this.hovered_row == Some(id) {
                    this.hovered_row = None;
                }
                cx.notify();
            }))
            .on_drag_move::<DraggedSubtask>(cx.listener(
                move |this, event: &DragMoveEvent<DraggedSubtask>, _, cx| {
                    this.drag_over_row(id, event, cx)
                },
            ))
            .child(
                canvas(
                    move |bounds: Bounds<Pixels>, _, _| {
                        bounds_cell.borrow_mut().insert(id, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(handle)
            .child(checkbox)
            .child(title)
            .children(actions)
    }
}

/// 进度卡片
fn progress_card(done: usize, total: usize) -> impl IntoElement {
    let percent = if total > 0 {
        (done as f32 / total as f32 * 100.).round()
    } else {
        0.
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(12.))
        .mb(px(16.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(10.))
        .bg(linear_gradient(
            135.,
            linear_color_stop(hex(editor_theme::SKY_50), 0.),
            linear_color_stop(hex(editor_theme::SKY_100), 1.),
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(50.))
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(theme::WEIGHT_SEMIBOLD)
                        .text_color(hex(editor_theme::SKY_700))
                        .child(format!("{done} / {total}")),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(light().text_secondary)
                        .child("已完成"),
                ),
        )
        .child(
            div()
                .flex_1()
                .h(px(6.))
                .rounded(px(3.))
                .bg(light().border)
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(relative(percent / 100.))
                        .rounded(px(3.))
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(light().primary, 0.),
                            linear_color_stop(hex(editor_theme::CYAN_500), 1.),
                        )),
                ),
        )
}
