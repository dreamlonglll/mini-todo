//! 清单窗口标题栏的清单名（PRD R6）：双击就地改名（同收纳箱：锁定中也允许）、右键菜单「重命名 / 删除清单」
//! （默认清单的「删除清单」置灰）、删除前确认"N 项待办将移到「<默认清单>」"。
//!
//! 右键菜单跟随本窗口的日 / 夜（`EpDropdownMenu::dark`）。
//!
//! - **双击**：标题栏挂了自绘移动（`BlockDrag::title_bar`：按下记点、超过 3px 才拖），不能用 `on_click` 判断双击（第二下
//!   手抖 2~3px 就收不到，gpui-conventions §2.3）。清单名的 `on_mouse_up` 问 `BlockDrag::is_double_click_release`：这次按下是
//!   双击的第二下、没拖动窗口、离按下点不超过 3px。清单名不 `occlude()`，在它上面按下照样能拖动窗口
//! - **改名**：Enter / 失焦提交，Esc 放弃；去首尾空白后与原名相同不写库；空名、超长、与其它清单重名（不区分大小写）时提示并
//!   保持原名（[`rename_outcome`]，规则同 core `lists::normalize_name` / `names_conflict`）；写库失败提示 core 的错误
//! - **删除**：后台数一下要移走的待办（`core::lists::todo_count`：存储的 `list_id` 等于本清单的，含已完成）→ 确认 →
//!   `AppState::delete_list`（core `lists::delete`：待办移到默认清单、写清单墓碑、删窗口记录）；成功后清单列表变化，
//!   `services::block_windows` 对账时关掉本窗口

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseUpEvent, ParentElement as _, SharedString, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use mini_todo_core::lists;

use super::ListWindow;
use super::title_bar::relative_line;
use crate::assets::EpIcon;
use crate::db::spawn_db;
use crate::theme::{self, Palette, main_window as dims};
use crate::ui::ep_dropdown::{EpDropdownItem, EpDropdownMenu};
use crate::ui::ep_popper::{self, Anchor, Placement, PopperOptions};
use crate::ui::input::EpInput;
use crate::ui::message;
use crate::ui::message_box::{self, BoxKind, ConfirmOptions};
use crate::windows::block::DEFAULT_LIST_ID;

/// 右键菜单的宽度（两项，最长"删除清单"：图标 14 + 5 + 4 字 × 14 + 左右内边距 32，取整留余量）
const MENU_WIDTH: f32 = 128.;

/// 清单名的改名 / 右键菜单状态（`ListWindow` 持有一份）
pub(crate) struct NameEditor {
    input: Entity<InputState>,
    renaming: bool,
    menu_open: bool,
    anchor: Anchor,
}

impl NameEditor {
    /// 建输入框（限 50 字，同 core）并订阅 Enter / 失焦提交；返回的订阅放进视图的 `_subscriptions`
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ListWindow>) -> (Self, Subscription) {
        let max_chars = lists::MAX_LIST_NAME_CHARS;
        let input = cx.new(|cx| {
            InputState::new(window, cx).validate(move |s, _| s.chars().count() <= max_chars)
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this: &mut ListWindow, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.finish_rename(true, window, cx)
                }
                _ => {}
            },
        );
        (
            Self {
                input,
                renaming: false,
                menu_open: false,
                anchor: Anchor::new(),
            },
            subscription,
        )
    }

    pub(crate) fn is_renaming(&self) -> bool {
        self.renaming
    }
}

/// 改名输入框里的内容该怎么处理
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RenameOutcome {
    /// 去首尾空白后与原名相同：什么都不做
    Unchanged,
    /// 写库（规范化后的名字）
    Rename(String),
    /// 不写库，提示这段文字并保持原名（空名、超长、重名）
    Rejected(String),
}

/// 改名的校验（core `lists::normalize_name` / `names_conflict` 的同一套规则，先在界面上拦下，提示文字与 core 相同）：
/// `others` 是其它清单的名字（不含本清单：只改大小写也算改名）
pub(crate) fn rename_outcome<'a>(
    draft: &str,
    current: &str,
    others: impl IntoIterator<Item = &'a str>,
) -> RenameOutcome {
    let name = match lists::normalize_name(draft) {
        Ok(name) => name,
        Err(e) => return RenameOutcome::Rejected(e),
    };
    if name == current {
        return RenameOutcome::Unchanged;
    }
    match others
        .into_iter()
        .find(|other| lists::names_conflict(other, &name))
    {
        Some(other) => RenameOutcome::Rejected(format!("已有同名清单「{}」", other.trim())),
        None => RenameOutcome::Rename(name),
    }
}

/// 删除清单的确认文字（PRD R1："N 项待办将移到「待办清单」"；默认清单可能已改名，用它现在的名字）
pub(crate) fn delete_message(name: &str, todos: usize, default_name: &str) -> String {
    if todos == 0 {
        format!("确定删除清单「{name}」吗？")
    } else {
        format!("确定删除清单「{name}」吗？{todos} 项待办将移到「{default_name}」。")
    }
}

impl ListWindow {
    /// 开始就地改名（双击清单名、右键「重命名」、托盘新建清单后）：输入框填原名、全选
    pub(crate) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.state.read(cx).list_name(self.list_id());
        self.name.renaming = true;
        self.name.menu_open = false;
        self.name.input.update(cx, |s, cx| {
            s.set_value(name, window, cx);
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        cx.notify();
    }

    /// 结束改名：`commit` 时按 [`rename_outcome`] 写库或提示（Esc 放弃时不提交）
    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.name.renaming {
            return;
        }
        self.name.renaming = false;
        cx.notify();
        if !commit {
            return;
        }
        let list_id = self.list_id();
        let draft = self.name.input.read(cx).value().to_string();
        let outcome = {
            let state = self.state.read(cx);
            let current = state.list_name(list_id);
            rename_outcome(
                &draft,
                &current,
                state
                    .lists
                    .iter()
                    .filter(|l| l.id != list_id)
                    .map(|l| l.name.as_str()),
            )
        };
        let name = match outcome {
            RenameOutcome::Unchanged => return,
            RenameOutcome::Rejected(text) => {
                message::warning(window, cx, text);
                return;
            }
            RenameOutcome::Rename(name) => name,
        };
        let task = self
            .state
            .update(cx, |state, cx| state.rename_list(list_id, name, cx));
        cx.spawn_in(window, async move |this, cx| {
            if let Err(e) = task.await {
                log::warn!("[list] 重命名清单 #{list_id} 失败: {e}");
                let _ = this.update_in(cx, |_, window, cx| {
                    message::error_with(window, cx, "重命名清单失败", &e)
                });
            }
        })
        .detach();
    }

    fn open_name_menu(&mut self, cx: &mut Context<Self>) {
        if !self.name.renaming && !self.name.menu_open {
            self.name.menu_open = true;
            cx.notify();
        }
    }

    fn close_name_menu(&mut self, cx: &mut Context<Self>) {
        if self.name.menu_open {
            self.name.menu_open = false;
            cx.notify();
        }
    }

    fn on_name_command(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_name_menu(cx);
        match command {
            "rename" => self.begin_rename(window, cx),
            "delete" => self.confirm_delete(window, cx),
            _ => {}
        }
    }

    /// 删除清单：数一下要移到默认清单的待办 → 确认 → 删除（默认清单不能删：菜单项置灰，这里再拦一次）
    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list_id = self.list_id();
        if list_id == DEFAULT_LIST_ID {
            return;
        }
        let count = spawn_db(cx, move |db| lists::todo_count(db, list_id));
        cx.spawn_in(window, async move |this, cx| {
            let count = count.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let count = match count {
                    Ok(count) => count,
                    Err(e) => {
                        message::error_with(window, cx, "删除清单失败", &e);
                        return;
                    }
                };
                let (name, default_name) = {
                    let state = this.state.read(cx);
                    (state.list_name(list_id), state.list_name(DEFAULT_LIST_ID))
                };
                let state = this.state.clone();
                message_box::confirm(
                    window,
                    cx,
                    ConfirmOptions::new("删除清单", delete_message(&name, count, &default_name))
                        .confirm_text("删除")
                        .cancel_text("取消")
                        .kind(BoxKind::Warning),
                    move |confirmed, window, cx| {
                        if !confirmed {
                            return;
                        }
                        let task = state.update(cx, |state, cx| state.delete_list(list_id, cx));
                        let handle = window.window_handle();
                        cx.spawn(async move |cx| match task.await {
                            Ok(moved) => log::info!(
                                "[list] 已删除清单 #{list_id}，{moved} 项待办移到默认清单"
                            ),
                            Err(e) => {
                                log::warn!("[list] 删除清单 #{list_id} 失败: {e}");
                                cx.update(|cx| {
                                    message::show_in(
                                        handle,
                                        cx,
                                        message::MessageKind::Error,
                                        message::error_text("删除清单失败", &e),
                                    )
                                });
                            }
                        })
                        .detach();
                    },
                );
            });
        })
        .detach();
    }

    /// 清单名：旧版 CSS 标题不换行——默认清单窗口不收缩、不折行（收缩的是后面的版本号）；其它清单窗口没有版本号，
    /// 名字太长时截断加省略号。改名中换成输入框
    pub(super) fn render_list_name(
        &mut self,
        name: String,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.name.renaming {
            return div()
                .id("list-name-edit")
                // 挡住标题栏：在输入框里按下、拖选文字不拖动窗口
                .occlude()
                .w(px(dims::LIST_NAME_INPUT_WIDTH))
                .min_w_0()
                .text_color(theme::ep().text_regular)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.finish_rename(false, window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(EpInput::new(&self.name.input).small().w_full())
                .into_any_element();
        }
        let default_list = self.list_id() == DEFAULT_LIST_ID;
        div()
            .id("list-name")
            .relative()
            .when(default_list, |this| this.flex_none())
            .when(!default_list, |this| this.min_w_0().truncate())
            .whitespace_nowrap()
            .text_size(px(16.))
            .line_height(relative_line(16.))
            .font_weight(theme::WEIGHT_SEMIBOLD)
            .text_color(palette.text_primary)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if this.window_drag.is_double_click_release(event.position) {
                        this.begin_rename(window, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| this.open_name_menu(cx)),
            )
            .child(name)
            .child(self.name.anchor.measure())
            .into_any_element()
    }

    /// 右键菜单（清单名下方、左对齐，跟随本窗口的日 / 夜）；关闭时不渲染。默认清单的「删除清单」置灰
    pub(super) fn render_name_menu(
        &mut self,
        dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.name.menu_open || self.name.renaming {
            return None;
        }
        let default_list = self.list_id() == DEFAULT_LIST_ID;
        let menu = EpDropdownMenu::new("list-name-menu")
            .dark(dark)
            .item(EpDropdownItem::new("rename", "重命名").icon(EpIcon::EditPen))
            .item(
                EpDropdownItem::new("delete", "删除清单")
                    .icon(EpIcon::Delete)
                    .disabled(default_list),
            )
            .on_select(cx.listener(|this, command: &SharedString, window, cx| {
                this.on_name_command(command, window, cx)
            }));
        let dismiss = cx.entity().downgrade();
        ep_popper::popper(
            &self.name.anchor,
            window,
            PopperOptions {
                placement: Placement::BottomStart,
                flip: false,
                gap: ep_popper::OFFSET,
                arrow: true,
                width: MENU_WIDTH,
                height: EpDropdownMenu::estimated_height(2, 0),
                dark,
            },
            div().w(px(MENU_WIDTH)).child(menu),
            move |_, cx| {
                let _ = dismiss.update(cx, |this, cx| this.close_name_menu(cx));
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renaming_trims_and_ignores_an_unchanged_name() {
        assert_eq!(
            rename_outcome("  工作  ", "待办清单", ["生活"]),
            RenameOutcome::Rename("工作".into())
        );
        assert_eq!(
            rename_outcome(" 工作 ", "工作", ["生活"]),
            RenameOutcome::Unchanged
        );
        // 只改大小写也是改名（自己不在 others 里）
        assert_eq!(
            rename_outcome("Work", "work", ["生活"]),
            RenameOutcome::Rename("Work".into())
        );
    }

    #[test]
    fn empty_too_long_and_duplicate_names_keep_the_old_name() {
        assert_eq!(
            rename_outcome("   ", "工作", []),
            RenameOutcome::Rejected("清单名称不能为空".into())
        );
        let long = "长".repeat(lists::MAX_LIST_NAME_CHARS + 1);
        assert!(matches!(
            rename_outcome(&long, "工作", []),
            RenameOutcome::Rejected(_)
        ));
        // 重名不区分大小写，提示用现有清单的名字
        assert_eq!(
            rename_outcome(" life ", "工作", ["待办清单", "Life"]),
            RenameOutcome::Rejected("已有同名清单「Life」".into())
        );
    }

    #[test]
    fn delete_message_counts_the_moved_todos() {
        assert_eq!(
            delete_message("工作", 3, "待办清单"),
            "确定删除清单「工作」吗？3 项待办将移到「待办清单」。"
        );
        assert_eq!(delete_message("空", 0, "收件箱"), "确定删除清单「空」吗？");
    }
}
