//! 日历窗口标题栏的清单筛选（PRD R7，设备本地，存在日历窗口的属性里）：漏斗按钮 + 下拉多选（`EpDropdownMenu` 的勾选项，
//! 跟随本窗口的日 / 夜）。
//!
//! 取值是"全部"（`None`，包含以后新建的清单）或一组清单 id：
//! - 菜单第一项「全部清单」，下面按清单顺序列出每个清单；"全部"时每一项都打勾
//! - 点「全部清单」→ 全部；点某个清单 → 勾上 / 取消它（从"全部"出发取消一个 = 其余的清单）；勾满了回到"全部"，
//!   一个都不剩也回到"全部"（[`next_filter`]）
//! - 选了具体清单时以后新建的清单不勾；被删除的清单自动移出、空了回到"全部"（`AppState::calendar_filter`）
//! - 兜底到默认清单的待办按默认清单参与筛选（`state::in_calendar_filter`）
//!
//! 点菜单项后菜单不关（多选）；点外面、再点按钮、窗口失活时关闭。不是"全部"时按钮显示为主色。

use gpui_kit::{AnyElement, Context, ParentElement as _, SharedString, Styled as _, Window, px};
use mini_todo_core::db::TodoList;

use super::CalendarWindow;
use crate::assets::EpIcon;
use crate::theme::Palette;
use crate::ui::ep_dropdown::{EpDropdownItem, EpDropdownMenu};
use crate::ui::ep_popper::{self, Placement, PopperOptions};
use crate::ui::message::{self, MessageKind};
use crate::windows::list_window::title_bar::title_button;

/// 「全部清单」的命令；清单项的命令是 `list:<清单 id>`
const ALL_COMMAND: &str = "all";
const LIST_PREFIX: &str = "list:";
/// 菜单宽度范围（按最长的清单名估算；更长的截断）与最高（清单很多时滚动）
const MENU_MIN_WIDTH: f32 = 120.;
const MENU_MAX_WIDTH: f32 = 220.;
const MENU_MAX_HEIGHT: f32 = 320.;

/// 菜单项对应的操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FilterCommand {
    /// 「全部清单」
    All,
    /// 勾上 / 取消一个清单
    Toggle(i64),
}

/// 菜单项的命令 → 操作
pub(crate) fn parse_command(command: &str) -> Option<FilterCommand> {
    if command == ALL_COMMAND {
        return Some(FilterCommand::All);
    }
    command
        .strip_prefix(LIST_PREFIX)?
        .parse()
        .ok()
        .map(FilterCommand::Toggle)
}

/// 点了菜单项之后的筛选。`current`：现在生效的筛选（`AppState::calendar_filter`，只含现有清单）；`lists`：现有清单的 id
pub(crate) fn next_filter(
    current: Option<&[i64]>,
    lists: &[i64],
    command: FilterCommand,
) -> Option<Vec<i64>> {
    let FilterCommand::Toggle(id) = command else {
        return None;
    };
    if !lists.contains(&id) {
        return current.map(<[i64]>::to_vec);
    }
    let mut ids: Vec<i64> = match current {
        None => lists.to_vec(),
        Some(current) => lists
            .iter()
            .copied()
            .filter(|l| current.contains(l))
            .collect(),
    };
    match ids.iter().position(|x| *x == id) {
        Some(index) => {
            ids.remove(index);
        }
        None => ids.push(id),
    }
    // 一个都不剩、或者勾满了：回到"全部"
    if ids.is_empty() || lists.iter().all(|l| ids.contains(l)) {
        return None;
    }
    ids.sort_unstable();
    Some(ids)
}

/// 菜单项：「全部清单」+ 分隔线 + 按清单顺序的每个清单（勾选状态按 `filter`）
pub(crate) fn menu_items(filter: Option<&[i64]>, lists: &[TodoList]) -> Vec<EpDropdownItem> {
    let all = filter.is_none();
    let mut items = vec![EpDropdownItem::new(ALL_COMMAND, "全部清单").checked(all)];
    items.extend(lists.iter().enumerate().map(|(index, list)| {
        let checked = all || filter.is_some_and(|ids| ids.contains(&list.id));
        let item = EpDropdownItem::new(format!("{LIST_PREFIX}{}", list.id), list.name.clone())
            .checked(checked);
        if index == 0 { item.divided() } else { item }
    }));
    items
}

impl CalendarWindow {
    fn toggle_filter_menu(&mut self, cx: &mut Context<Self>) {
        self.filter_open = !self.filter_open;
        cx.notify();
    }

    pub(super) fn close_filter_menu(&mut self, cx: &mut Context<Self>) {
        if self.filter_open {
            self.filter_open = false;
            cx.notify();
        }
    }

    fn apply_filter_command(&mut self, command: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(command) = parse_command(command) else {
            return;
        };
        let (current, next) = {
            let state = self.state.read(cx);
            let current = state.calendar_filter();
            let lists: Vec<i64> = state.lists.iter().map(|l| l.id).collect();
            let next = next_filter(current.as_deref(), &lists, command);
            (current, next)
        };
        if next == current {
            return;
        }
        let task = self
            .state
            .update(cx, |state, cx| state.set_calendar_filter(next, cx));
        let handle = window.window_handle();
        cx.spawn(async move |_, cx| {
            if let Err(text) = task.await {
                cx.update(|cx| message::show_in(handle, cx, MessageKind::Error, text));
            }
        })
        .detach();
    }

    /// 标题栏的筛选按钮（漏斗；不是"全部"时主色），下拉菜单挂在它下面
    pub(super) fn filter_button(
        &self,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        let active = self.state.read(cx).calendar_filter().is_some();
        let weak = cx.entity().downgrade();
        title_button(
            "calendar-filter",
            EpIcon::Filter,
            "清单筛选",
            active,
            palette,
            move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| this.toggle_filter_menu(cx));
            },
        )
        .relative()
        .child(self.filter_anchor.measure())
    }

    /// 筛选下拉（按钮下方、右对齐、带箭头）；关闭时不渲染。`dark`：本窗口是深色主题
    pub(super) fn render_filter_menu(
        &mut self,
        dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.filter_open {
            return None;
        }
        let items = {
            let state = self.state.read(cx);
            menu_items(state.calendar_filter().as_deref(), &state.lists)
        };
        let menu = items
            .into_iter()
            .fold(
                EpDropdownMenu::new("calendar-filter-menu")
                    .dark(dark)
                    .max_height(px(MENU_MAX_HEIGHT)),
                |menu, item| menu.item(item),
            )
            .on_select(cx.listener(|this, command: &SharedString, window, cx| {
                this.apply_filter_command(command, window, cx)
            }));
        let width = menu.panel_width().clamp(MENU_MIN_WIDTH, MENU_MAX_WIDTH);
        let height = menu.panel_height();
        let dismiss = cx.entity().downgrade();
        ep_popper::popper(
            &self.filter_anchor,
            window,
            PopperOptions {
                placement: Placement::BottomEnd,
                flip: false,
                gap: ep_popper::OFFSET,
                arrow: true,
                width,
                height,
                dark,
            },
            menu.width(width),
            move |_, cx| {
                // 点在按钮上不会走到这里（交给按钮自己切换）
                let _ = dismiss.update(cx, |this, cx| this.close_filter_menu(cx));
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTS: [i64; 3] = [1, 7, 8];

    fn list(id: i64, name: &str) -> TodoList {
        TodoList {
            id,
            name: name.into(),
            sort_order: id,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn commands_round_trip() {
        assert_eq!(parse_command("all"), Some(FilterCommand::All));
        assert_eq!(
            parse_command("list:1731000000000123"),
            Some(FilterCommand::Toggle(1_731_000_000_000_123))
        );
        assert_eq!(parse_command("list:x"), None);
        assert_eq!(parse_command("move"), None);
    }

    #[test]
    fn unchecking_from_all_keeps_the_other_lists() {
        use FilterCommand::*;
        // "全部"时取消一个：其余清单（以后新建的清单不在里面）
        assert_eq!(next_filter(None, &LISTS, Toggle(7)), Some(vec![1, 8]));
        assert_eq!(next_filter(Some(&[1, 8]), &LISTS, Toggle(8)), Some(vec![1]));
        // 勾回来：按 id 排序
        assert_eq!(next_filter(Some(&[8]), &LISTS, Toggle(1)), Some(vec![1, 8]));
        // 「全部清单」：回到全部
        assert_eq!(next_filter(Some(&[1]), &LISTS, All), None);
        assert_eq!(next_filter(None, &LISTS, All), None);
    }

    #[test]
    fn empty_or_full_selections_fall_back_to_all() {
        use FilterCommand::*;
        // 取消最后一个：空了回到全部
        assert_eq!(next_filter(Some(&[7]), &LISTS, Toggle(7)), None);
        // 勾满了：就是全部（之后新建的清单也显示）
        assert_eq!(next_filter(Some(&[1, 8]), &LISTS, Toggle(7)), None);
        // 只有默认清单时取消它：还是全部
        assert_eq!(next_filter(None, &[1], Toggle(1)), None);
        // 已不存在的清单：不变
        assert_eq!(next_filter(Some(&[7]), &LISTS, Toggle(99)), Some(vec![7]));
        assert_eq!(next_filter(None, &LISTS, Toggle(99)), None);
    }

    #[test]
    fn menu_checks_every_list_when_showing_all() {
        let lists = vec![list(1, "待办清单"), list(7, "工作"), list(8, "生活")];
        let checked = |filter: Option<&[i64]>| -> Vec<bool> {
            menu_items(filter, &lists)
                .iter()
                .map(|i| i.checked)
                .collect()
        };
        assert_eq!(checked(None), vec![true, true, true, true]);
        assert_eq!(checked(Some(&[7])), vec![false, false, true, false]);
        let items = menu_items(None, &lists);
        assert_eq!(items[0].label.as_ref(), "全部清单");
        assert!(
            items[1].divided && !items[2].divided,
            "「全部清单」下面一条分隔线"
        );
        assert_eq!(
            parse_command(&items[3].command),
            Some(FilterCommand::Toggle(8))
        );
    }
}
