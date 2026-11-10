//! 待办行的右键菜单（PRD R8，列表与四象限共用）：只有一项「移动到 ▸」，悬停或点击展开级联子菜单，列出其它清单
//! （按清单顺序，不含本清单）；只有默认清单时置灰并提示"没有其它清单"。菜单在按下的位置弹出（`Anchor::at`），
//! 子菜单贴在它旁边（`Placement::RightStart`，右边放不下翻到左边）；样式用 `EpDropdownMenu`，跟随本窗口的日 / 夜。
//!
//! 选中目标 → `AppState::move_todo_to_list`（本地立即生效：这条随即从本窗口消失、出现在目标清单的窗口里；失败在本窗口
//! 提示 `移动待办失败：…`）。不做跨窗口拖动。
//!
//! 关闭：点菜单外面（点在子菜单上不算）、选中目标、这条待办已不是本清单的未完成待办（被移走 / 完成 / 删除 / 同步改了
//! 归属，见 `ListWindow::on_state_changed`）、切换视图、窗口失活。

use gpui_kit::{AnyElement, Context, Pixels, Point, SharedString, Window, px};
use mini_todo_core::db::TodoList;

use super::ListWindow;
use crate::assets::EpIcon;
use crate::ui::ep_dropdown::{EpDropdownItem, EpDropdownMenu};
use crate::ui::ep_popper::{self, Anchor, Placement, PopperOptions};

/// 主菜单唯一一项的命令
const MOVE_COMMAND: &str = "move";
/// 子菜单项命令的前缀（`list:<清单 id>`）
const TARGET_PREFIX: &str = "list:";
/// 没有其它清单时的说明（置灰项右侧与悬停提示）
pub(crate) const NO_OTHER_LISTS: &str = "没有其它清单";
/// 菜单与按下位置的距离
const CURSOR_GAP: f32 = 2.;
/// 子菜单与主菜单的距离
const SUBMENU_GAP: f32 = 4.;
/// 子菜单的宽度范围（按最长的清单名估算；更长的截断加省略号）与最高（清单很多时滚动）
const SUBMENU_MIN_WIDTH: f32 = 120.;
const SUBMENU_MAX_WIDTH: f32 = 220.;
const SUBMENU_MAX_HEIGHT: f32 = 320.;

/// 打开中的右键菜单
pub(crate) struct RowMenu {
    todo_id: i64,
    /// 右键按下的位置（零尺寸锚点：菜单左上角贴着它，下面放不下时底边贴着它）
    cursor: Anchor,
    /// 主菜单的外框（子菜单贴在它旁边；点在它上面不收起子菜单）
    frame: Anchor,
    /// 子菜单（其它清单）展开着；展开后一直开到整个菜单关闭（主菜单只有一项）
    submenu_open: bool,
    /// 子菜单的外框（点在它上面不算点了主菜单外面）
    submenu_frame: Anchor,
}

impl RowMenu {
    fn new(todo_id: i64, position: Point<Pixels>) -> Self {
        Self {
            todo_id,
            cursor: Anchor::at(position),
            frame: Anchor::new(),
            submenu_open: false,
            submenu_frame: Anchor::new(),
        }
    }

    /// 菜单是给哪条待办开的
    pub(crate) fn todo_id(&self) -> i64 {
        self.todo_id
    }
}

/// 「移动到」的目标：其它清单（按清单顺序，不含待办现在的清单 `current`）
pub(crate) fn move_targets(lists: &[TodoList], current: i64) -> Vec<&TodoList> {
    lists.iter().filter(|l| l.id != current).collect()
}

/// 主菜单唯一的一项：有目标时带级联箭头；没有时置灰并说明"没有其它清单"
pub(crate) fn move_item(has_targets: bool) -> EpDropdownItem {
    let item = EpDropdownItem::new(MOVE_COMMAND, "移动到")
        .icon(EpIcon::Rank)
        .submenu();
    if has_targets {
        item
    } else {
        item.disabled(true).note(NO_OTHER_LISTS)
    }
}

/// 子菜单的一项（命令 `list:<清单 id>`，文字是清单名）
pub(crate) fn target_item(list: &TodoList) -> EpDropdownItem {
    EpDropdownItem::new(format!("{TARGET_PREFIX}{}", list.id), list.name.clone())
}

/// 子菜单项的命令 → 目标清单 id
pub(crate) fn parse_target(command: &str) -> Option<i64> {
    command.strip_prefix(TARGET_PREFIX)?.parse().ok()
}

impl ListWindow {
    /// 右键待办行：在按下的位置打开菜单（拖拽排序中不开；清单名的菜单先关掉）
    pub(crate) fn open_row_menu(
        &mut self,
        todo_id: i64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.drag.as_ref().is_some_and(|d| !d.dropped) || cx.has_active_drag() {
            return;
        }
        self.close_name_menu(cx);
        self.row_menu = Some(RowMenu::new(todo_id, position));
        cx.notify();
    }

    pub(crate) fn close_row_menu(&mut self, cx: &mut Context<Self>) {
        if self.row_menu.take().is_some() {
            cx.notify();
        }
    }

    fn open_move_submenu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.row_menu.as_mut()
            && !menu.submenu_open
        {
            menu.submenu_open = true;
            cx.notify();
        }
    }

    fn move_todo(&mut self, todo_id: i64, list_id: i64, window: &Window, cx: &mut Context<Self>) {
        self.close_row_menu(cx);
        let task = self.state.update(cx, |state, cx| {
            state.move_todo_to_list(todo_id, list_id, cx)
        });
        self.notify_on_error(task, "移动待办失败", window, cx);
    }

    /// 右键菜单（主菜单 + 展开时的子菜单，画在本帧最后的浮层）；`dark`：本窗口是深色主题
    pub(super) fn render_row_menu(
        &mut self,
        dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(menu) = self.row_menu.as_ref() else {
            return Vec::new();
        };
        let todo_id = menu.todo_id;
        let targets: Vec<TodoList> = move_targets(&self.state.read(cx).lists, self.list_id())
            .into_iter()
            .cloned()
            .collect();
        let mut layers = Vec::with_capacity(2);

        let weak = cx.entity().downgrade();
        let main = EpDropdownMenu::new("todo-row-menu")
            .dark(dark)
            .anchor(menu.frame.clone())
            .item(move_item(!targets.is_empty()))
            .on_hover(move |command, hovered, _, cx| {
                if hovered && command.as_ref() == MOVE_COMMAND {
                    let _ = weak.update(cx, |this, cx| this.open_move_submenu(cx));
                }
            })
            .on_select(cx.listener(|this, command: &SharedString, _, cx| {
                if command.as_ref() == MOVE_COMMAND {
                    this.open_move_submenu(cx);
                }
            }));
        let (width, height) = (main.panel_width(), main.panel_height());
        let submenu_frame = menu.submenu_frame.clone();
        let dismiss = cx.entity().downgrade();
        layers.extend(ep_popper::popper(
            &menu.cursor,
            window,
            PopperOptions {
                placement: Placement::BottomStart,
                flip: true,
                gap: CURSOR_GAP,
                arrow: false,
                width,
                height,
                dark,
            },
            main.width(width),
            move |window, cx| {
                // 主菜单的"外面"包括子菜单：点在子菜单上交给子菜单
                if submenu_frame.contains(window.mouse_position()) {
                    return;
                }
                let _ = dismiss.update(cx, |this, cx| this.close_row_menu(cx));
            },
        ));

        if menu.submenu_open && !targets.is_empty() {
            let sub = targets
                .iter()
                .fold(
                    EpDropdownMenu::new("todo-row-submenu")
                        .dark(dark)
                        .anchor(menu.submenu_frame.clone())
                        .max_height(px(SUBMENU_MAX_HEIGHT)),
                    |sub, list| sub.item(target_item(list)),
                )
                .on_select(
                    cx.listener(move |this, command: &SharedString, window, cx| {
                        if let Some(list_id) = parse_target(command) {
                            this.move_todo(todo_id, list_id, window, cx);
                        }
                    }),
                );
            let width = sub
                .panel_width()
                .clamp(SUBMENU_MIN_WIDTH, SUBMENU_MAX_WIDTH);
            let height = sub.panel_height();
            let dismiss = cx.entity().downgrade();
            // 锚点是主菜单的外框：点在主菜单上不收起（popper 不把锚点算作外面）
            layers.extend(ep_popper::popper(
                &menu.frame,
                window,
                PopperOptions {
                    placement: Placement::RightStart,
                    flip: true,
                    gap: SUBMENU_GAP,
                    arrow: false,
                    width,
                    height,
                    dark,
                },
                sub.width(width),
                move |_, cx| {
                    let _ = dismiss.update(cx, |this, cx| this.close_row_menu(cx));
                },
            ));
        }
        layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(id: i64, name: &str, sort_order: i64) -> TodoList {
        TodoList {
            id,
            name: name.into(),
            sort_order,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn targets_are_the_other_lists_in_list_order() {
        let lists = vec![
            list(1, "待办清单", 0),
            list(7, "工作", 1),
            list(8, "生活", 2),
        ];
        let names = |current| -> Vec<&str> {
            move_targets(&lists, current)
                .iter()
                .map(|l| l.name.as_str())
                .collect()
        };
        assert_eq!(names(1), vec!["工作", "生活"]);
        // 其它清单的窗口：默认清单也是目标
        assert_eq!(names(7), vec!["待办清单", "生活"]);
        assert!(move_targets(&lists[..1], 1).is_empty(), "只有默认清单");
    }

    #[test]
    fn move_item_is_greyed_out_without_other_lists() {
        let item = move_item(true);
        assert!(item.submenu && !item.disabled && item.note.is_none());
        assert_eq!(item.label.as_ref(), "移动到");
        let item = move_item(false);
        assert!(item.disabled);
        assert_eq!(item.note.as_ref().map(|n| n.as_ref()), Some(NO_OTHER_LISTS));
    }

    #[test]
    fn target_commands_round_trip_the_list_id() {
        let item = target_item(&list(1_731_000_000_000_123, "工作", 1));
        assert_eq!(item.label.as_ref(), "工作");
        assert_eq!(parse_target(&item.command), Some(1_731_000_000_000_123));
        assert_eq!(parse_target(MOVE_COMMAND), None);
        assert_eq!(parse_target("list:x"), None);
    }
}
