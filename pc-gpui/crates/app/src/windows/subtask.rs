//! 子任务查看 / 编辑窗口（旧 `SubtaskEditorView.vue`）。规格：`research/ui-editor-windows.md` §4、§3.13。
//!
//! 窗口 800×750、可缩放、不透明，打开方是编辑窗口（owner + 模态遮罩）。两种数据来源：
//! - 已落库的子任务：`core::todo::get_subtask` 加载，保存走 `AppState::update_subtask`；
//! - 新建待办里尚未落库的暂存子任务（负数 id）：编辑窗口持有 [`DraftSubtasks`]，这里直接读写同一个实体
//!   （PRD D5：旧版的 `subtask-memory-ready / -init / -save` 事件握手改为同进程直接共享）。
//!
//! 布局：标题栏（53 高，最大化 + 关闭）→ 内容（标题输入 + Markdown 内容，`gap 16`）→ 底部栏（右对齐 default
//! 尺寸按钮，间距 20）。Esc 关闭（有未保存修改时确认）、Ctrl/Cmd+Enter 保存（只读时无效）。

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use mini_todo_core::db::UpdateSubTaskRequest;

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size};
use crate::assets::EpIcon;
use crate::db::spawn_db;
use crate::platform;
use crate::state::AppState;
use crate::theme::{self, editor_theme, hex, light};
use crate::ui::button::EpButton;
use crate::ui::icon::icon;
use crate::ui::input::EpInput;
use crate::ui::markdown_editor::{MarkdownEditor, MarkdownEditorEvent};
use crate::ui::markdown_image_viewer::ImageViewer;
use crate::ui::title_bar::drag_region;
use crate::ui::tooltip::native;
use crate::ui::{message, message_box};
use crate::util::editor_shortcuts::{self, EditorClose, EditorSubmit};
use crate::util::image_ref;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubtaskMode {
    #[default]
    View,
    Edit,
}

impl SubtaskMode {
    pub fn parse(value: &str) -> Option<SubtaskMode> {
        match value {
            "view" => Some(SubtaskMode::View),
            "edit" => Some(SubtaskMode::Edit),
            _ => None,
        }
    }

    /// 系统窗口标题 / 标题栏文字
    pub fn title(self) -> &'static str {
        match self {
            SubtaskMode::View => "查看子任务",
            SubtaskMode::Edit => "编辑子任务",
        }
    }
}

pub const WIDTH: f32 = 800.;
pub const HEIGHT: f32 = 750.;

// ----------------------------------------------------------------------
// 新建待办里的暂存子任务
// ----------------------------------------------------------------------

/// 一条暂存子任务（id 为负数：-1, -2, …）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSubtask {
    pub id: i64,
    pub title: String,
    /// 存储形式的 Markdown；空 = `None`
    pub content: Option<String>,
    pub completed: bool,
}

/// 新建待办（尚未入库）时的子任务列表，编辑窗口与子任务窗口共享
#[derive(Debug, Default)]
pub struct DraftSubtasks {
    pub items: Vec<DraftSubtask>,
    counter: i64,
}

impl DraftSubtasks {
    /// 追加一条，返回新 id
    pub fn add(&mut self, title: String) -> i64 {
        self.counter -= 1;
        let id = self.counter;
        self.items.push(DraftSubtask {
            id,
            title,
            content: None,
            completed: false,
        });
        id
    }

    pub fn get(&self, id: i64) -> Option<&DraftSubtask> {
        self.items.iter().find(|s| s.id == id)
    }

    pub fn remove(&mut self, id: i64) {
        self.items.retain(|s| s.id != id);
    }

    pub fn toggle(&mut self, id: i64) {
        if let Some(item) = self.items.iter_mut().find(|s| s.id == id) {
            item.completed = !item.completed;
        }
    }

    /// 子任务窗口保存（旧 `subtask-memory-save`：内容 `''` → `None`）
    pub fn save(&mut self, id: i64, title: String, content: String) {
        if let Some(item) = self.items.iter_mut().find(|s| s.id == id) {
            item.title = title;
            item.content = (!content.is_empty()).then_some(content);
        }
    }

    /// 按给定顺序重排（不在列表里的 id 忽略）
    pub fn reorder(&mut self, ids: &[i64]) {
        self.items
            .sort_by_key(|s| ids.iter().position(|id| *id == s.id).unwrap_or(usize::MAX));
    }
}

// ----------------------------------------------------------------------
// 窗口
// ----------------------------------------------------------------------

/// 数据来源
enum Source {
    Db(i64),
    Draft {
        drafts: Entity<DraftSubtasks>,
        id: i64,
    },
}

/// 子任务窗口视图
pub struct SubtaskWindow {
    source: Source,
    mode: SubtaskMode,
    title: Entity<InputState>,
    content: Entity<MarkdownEditor>,
    saving: bool,
    clean_snapshot: String,
    focus: FocusHandle,
    image_viewer: Option<Entity<ImageViewer>>,
    _subscriptions: Vec<Subscription>,
}

impl SubtaskWindow {
    fn new(source: Source, mode: SubtaskMode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let images_dir: Arc<PathBuf> = Arc::new(mini_todo_core::db::paths::images_dir());
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("请输入子任务标题")
                .validate(|text, _| text.chars().count() <= 200)
        });
        let readonly = mode == SubtaskMode::View;
        let content = cx.new(|cx| {
            let mut editor = MarkdownEditor::new("subtask-content", images_dir, None, window, cx);
            editor.set_padding(12., 16.);
            editor.set_readonly(readonly, cx);
            editor
        });
        let subscriptions = vec![
            cx.subscribe_in(&title, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&content, window, |_, _, _: &MarkdownEditorEvent, _, cx| {
                cx.notify()
            }),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            source,
            mode,
            title,
            content,
            saving: false,
            clean_snapshot: String::new(),
            focus,
            image_viewer: None,
            _subscriptions: subscriptions,
        };
        this.install_image_click(cx);
        // 先按空值拍一次（加载完成前按 Esc 不会误报）
        this.clean_snapshot = this.snapshot(cx);
        this.load(window, cx);
        this
    }

    fn is_view_mode(&self) -> bool {
        self.mode == SubtaskMode::View
    }

    /// `[title.trim(), toStorageMarkdown(content)]`
    fn snapshot(&self, cx: &App) -> String {
        serde_json::json!([
            self.title.read(cx).value().trim(),
            self.content.read(cx).value(cx)
        ])
        .to_string()
    }

    fn fill(&mut self, title: &str, content: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.title.update(cx, |state, cx| {
            state.set_value(title.to_string(), window, cx)
        });
        self.content
            .update(cx, |editor, cx| editor.set_value(content, window, cx));
        self.clean_snapshot = self.snapshot(cx);
        cx.notify();
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.source {
            Source::Draft { drafts, id } => {
                let draft = drafts.read(cx).get(*id).cloned();
                if let Some(draft) = draft {
                    self.fill(
                        &draft.title,
                        draft.content.as_deref().unwrap_or_default(),
                        window,
                        cx,
                    );
                }
            }
            Source::Db(id) => {
                let id = *id;
                let task = spawn_db(cx, move |db| mini_todo_core::todo::get_subtask(db, id));
                cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
                    let result = task.await;
                    let _ = this.update_in(cx, |this, window, cx| match result {
                        Ok(subtask) => this.fill(
                            &subtask.title,
                            subtask.content.as_deref().unwrap_or_default(),
                            window,
                            cx,
                        ),
                        Err(e) => message::error_with(window, cx, "加载子任务失败", &e),
                    });
                })
                .detach();
            }
        }
    }

    fn install_image_click(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let handler: crate::ui::markdown::ImageClickHandler =
            std::rc::Rc::new(move |paths, index, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if paths.is_empty() {
                        return;
                    }
                    let viewer = cx.new(|cx| ImageViewer::new(paths, index, window, cx));
                    this._subscriptions
                        .push(cx.observe(&viewer, |this, viewer, cx| {
                            if viewer.read(cx).is_closed() {
                                this.image_viewer = None;
                                cx.notify();
                            }
                        }));
                    this.image_viewer = Some(viewer);
                    cx.notify();
                });
            });
        self.content
            .update(cx, |editor, _| editor.set_on_image_click(handler));
    }

    /// 旧 `handleSave()`
    fn handle_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.title.read(cx).value().trim().to_string();
        if self.is_view_mode() || self.saving || title.is_empty() {
            return;
        }
        let content = image_ref::to_storage_markdown(&self.content.read(cx).value(cx));
        match &self.source {
            Source::Draft { drafts, id } => {
                let id = *id;
                drafts.update(cx, |drafts, cx| {
                    drafts.save(id, title, content);
                    cx.notify();
                });
                window.remove_window();
            }
            Source::Db(id) => {
                let id = *id;
                self.saving = true;
                cx.notify();
                let request = UpdateSubTaskRequest {
                    title: Some(title),
                    content: Some(content),
                    completed: None,
                    sort_order: None,
                };
                let task = AppState::global(cx)
                    .update(cx, |state, cx| state.update_subtask(id, request, cx));
                cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
                    let result = task.await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.saving = false;
                        match result {
                            Ok(_) => window.remove_window(),
                            Err(e) => message::error_with(window, cx, "保存子任务失败", &e),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    fn on_close_action(&mut self, _: &EditorClose, window: &mut Window, cx: &mut Context<Self>) {
        if message_box::is_open(window, cx) {
            return;
        }
        if let Some(viewer) = self.image_viewer.take() {
            viewer.update(cx, |v, cx| v.close(cx));
            cx.notify();
            return;
        }
        if self.saving {
            return;
        }
        if !self.is_view_mode() && self.snapshot(cx) != self.clean_snapshot {
            message_box::confirm(
                window,
                cx,
                message_box::ConfirmOptions::new("放弃修改", "有未保存的修改，确定放弃并关闭吗？")
                    .confirm_text("放弃")
                    .cancel_text("继续编辑")
                    .kind(message_box::BoxKind::Warning),
                |confirmed, window, _| {
                    if confirmed {
                        window.remove_window();
                    }
                },
            );
            return;
        }
        window.remove_window();
    }

    fn on_submit_action(&mut self, _: &EditorSubmit, window: &mut Window, cx: &mut Context<Self>) {
        if self.image_viewer.is_some() || message_box::is_open(window, cx) {
            return;
        }
        self.handle_save(window, cx);
    }

    fn render_header(&self) -> impl IntoElement {
        let palette = light();
        let control = |id: &'static str, source: EpIcon, tip: &'static str, danger: bool| {
            div()
                .id(id)
                .group(id)
                .w(px(32.))
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .occlude()
                .hover(move |s| {
                    s.bg(if danger {
                        hex(editor_theme::RED_100)
                    } else {
                        palette.bg_tertiary
                    })
                })
                .tooltip(native(tip))
                .child(
                    icon(source, 14., palette.text_secondary).group_hover(id, move |s| {
                        s.text_color(if danger {
                            palette.error
                        } else {
                            hex(editor_theme::SLATE_700)
                        })
                    }),
                )
        };
        div()
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.))
            .py(px(12.))
            .min_h(px(44.))
            .border_b_1()
            .border_color(palette.border)
            .child(drag_region())
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .line_height(relative(1.2))
                    .text_color(palette.text_primary)
                    .child(self.mode.title()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        control("subtask-maximize", EpIcon::FullScreen, "最大化", false).on_click(
                            |_: &ClickEvent, window, cx| platform::toggle_maximize(window, cx),
                        ),
                    )
                    .child(
                        control("subtask-close", EpIcon::Close, "关闭", true)
                            .on_click(|_: &ClickEvent, window, _| window.remove_window()),
                    ),
            )
    }
}

impl gpui_kit::Focusable for SubtaskWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SubtaskWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let view_mode = self.is_view_mode();
        let label = |text: &'static str| {
            div()
                .text_size(px(13.))
                .font_weight(theme::WEIGHT_SEMIBOLD)
                .text_color(hex(editor_theme::SLATE_700))
                .child(text)
        };
        let title_empty = self.title.read(cx).value().trim().is_empty();
        let content = div()
            .flex_1()
            .min_h_0()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("标题"))
                    .child(EpInput::new(&self.title).disabled(view_mode).w_full()),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("内容 (Markdown)"))
                    .child(
                        div()
                            .id("subtask-content-box")
                            .flex_1()
                            .min_h(px(300.))
                            .overflow_y_scroll()
                            .border_1()
                            .border_color(palette.border)
                            .rounded(px(8.))
                            .bg(hex(0xFFFFFF))
                            // 至少撑满内容框（编辑器根元素再撑满这一层：点框内任意位置都进入编辑），内容更长时
                            // 随内容增高、在框内滚动（不能 `size_full`：滚动范围只按直接子元素算，长内容会滚不动）
                            .child(
                                div()
                                    .w_full()
                                    .min_h_full()
                                    .flex()
                                    .flex_col()
                                    .child(self.content.clone()),
                            ),
                    ),
            );
        let footer = div()
            .flex_none()
            .flex()
            .justify_end()
            .items_center()
            .gap(px(20.))
            .px(px(16.))
            .py(px(12.))
            .border_t_1()
            .border_color(palette.border)
            .child(
                EpButton::new("subtask-cancel")
                    .icon(EpIcon::Close)
                    .icon_gap(6.)
                    .label(if view_mode { "关闭" } else { "取消" })
                    .on_click(|_, window, _| window.remove_window()),
            )
            .when(!view_mode, |this| {
                this.child(
                    EpButton::new("subtask-save")
                        .primary()
                        .icon_gap(6.)
                        .label("保存")
                        .loading(self.saving)
                        .disabled(title_empty || self.saving)
                        .when(!self.saving, |this| this.icon(EpIcon::Check))
                        .on_click(cx.listener(|this, _, window, cx| this.handle_save(window, cx))),
                )
            });
        let viewer = self.image_viewer.clone();
        div()
            .id("subtask-window")
            .key_context(editor_shortcuts::EDITOR_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_close_action))
            .on_action(cx.listener(Self::on_submit_action))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .text_color(palette.text_primary)
            .bg(hex(0xFFFFFF))
            .child(self.render_header())
            .child(content)
            .child(footer)
            .children(viewer.map(|v| div().absolute().inset_0().child(v)))
    }
}

fn open_with(
    cx: &mut App,
    parent: Option<ChildOf>,
    source: Source,
    mode: SubtaskMode,
) -> Option<AnyWindowHandle> {
    editor_shortcuts::init(cx);
    let spec = WindowSpec::new(
        mode.title(),
        gpui_kit::Bounds::new(Default::default(), logical_size(WIDTH, HEIGHT)),
    );
    WindowManager::open_child(
        cx,
        parent.as_ref(),
        WindowKind::Subtask,
        None,
        spec,
        |window, cx| cx.new(|cx| SubtaskWindow::new(source, mode, window, cx)),
    )
}

/// 打开已落库子任务的窗口（打开方：编辑窗口 `ChildOf::window(window, cx)`；截图调试 `None`）
pub fn open(
    cx: &mut App,
    parent: Option<ChildOf>,
    subtask_id: i64,
    mode: SubtaskMode,
) -> Option<AnyWindowHandle> {
    open_with(cx, parent, Source::Db(subtask_id), mode)
}

/// 打开新建待办里暂存子任务的窗口（直接读写编辑窗口的 [`DraftSubtasks`]）
pub fn open_draft(
    cx: &mut App,
    parent: Option<ChildOf>,
    drafts: Entity<DraftSubtasks>,
    pending_id: i64,
    mode: SubtaskMode,
) -> Option<AnyWindowHandle> {
    open_with(
        cx,
        parent,
        Source::Draft {
            drafts,
            id: pending_id,
        },
        mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_use_negative_ids_and_keep_order() {
        let mut drafts = DraftSubtasks::default();
        let a = drafts.add("a".into());
        let b = drafts.add("b".into());
        let c = drafts.add("c".into());
        assert_eq!((a, b, c), (-1, -2, -3));
        drafts.toggle(b);
        assert!(drafts.get(b).unwrap().completed);
        drafts.reorder(&[c, a, b]);
        let ids: Vec<i64> = drafts.items.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![c, a, b]);
        drafts.save(a, "A".into(), String::new());
        assert_eq!(drafts.get(a).unwrap().content, None);
        drafts.save(a, "A".into(), "x".into());
        assert_eq!(drafts.get(a).unwrap().content.as_deref(), Some("x"));
        drafts.remove(c);
        assert_eq!(drafts.items.len(), 2);
        // 删除后再加不复用 id
        assert_eq!(drafts.add("d".into()), -4);
    }
}
