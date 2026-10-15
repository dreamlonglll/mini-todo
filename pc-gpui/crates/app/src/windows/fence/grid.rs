//! 项目网格（旧 `FenceItemGrid.vue`，规格 §4）：图标懒加载（同时最多 3 个请求、可见区上下外扩 64px、同一
//! `路径|修改时间|档位` 只请求一次）、单选与双击打开、自绘深色右键菜单（主页 / 「移动到…」页）、箱内拖动排序、
//! Ctrl+滚轮调图标大小（40ms 节流、400ms 防抖落库）。
//!
//! 布局：单元宽 = 图标 + 40，从左上排起、右侧空白不拉伸，行间 2px、列间 0（CSS `grid auto-fill` 的等价物：
//! 固定宽度单元的 flex-wrap，同一行等高）。默认 368 宽的收纳箱内容区 350 < 4 × 88，与旧版实际渲染一样排 3 列。

use std::path::PathBuf;
use std::time::Instant;

use gpui_kit::{
    Anchor, AnchoredPositionMode, AnyElement, AppContext as _, Context, DragMoveEvent, ImageSource,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ObjectFit,
    ParentElement as _, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, anchored, deferred,
    div, img, prelude::FluentBuilder as _, px,
};
use mini_todo_core::db::Fence;
use mini_todo_core::fence::items::{self, FenceEntry, ItemOpResult, ItemOpStatus};

use super::logic::{self, timing};
use super::{ExpandReason, FenceWindow};
use crate::assets::EpIcon;
use crate::db::spawn_db;
use crate::platform;
use crate::services::fences as runtime;
use crate::theme::{self, fence_tokens};
use crate::ui::icon::{icon, spinning};
use crate::ui::message;

/// 图标状态
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum IconState {
    Loading,
    Ready(PathBuf),
    Failed,
}

/// 排队中的图标请求
#[derive(Debug, Clone)]
pub(crate) struct IconRequest {
    key: String,
    file_name: String,
    size_px: u32,
}

/// 右键菜单页
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuPage {
    Main,
    Move,
}

/// 打开中的右键菜单
#[derive(Debug, Clone)]
pub(crate) struct ItemMenu {
    position: Point<Pixels>,
    item: FenceEntry,
    page: MenuPage,
    targets: Vec<Fence>,
}

/// 箱内拖动排序的拖动值
#[derive(Clone)]
struct FenceItemDrag {
    file_name: String,
}

/// 拖动排序时跟随鼠标的单元克隆（SortableJS fallback 克隆体：按下即选中，所以带选中样式，`opacity .8`）
struct FenceItemDragView {
    entry: FenceEntry,
    icon: Option<PathBuf>,
    failed: bool,
    icon_size: f32,
}

impl Render for FenceItemDragView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = fence_tokens();
        render_cell_body(
            &self.entry,
            self.icon.as_ref(),
            self.failed,
            false,
            self.icon_size,
        )
        .relative()
        .w(px(self.icon_size + 40.))
        .pt(px(6.))
        .px(px(4.))
        .pb(px(4.))
        .rounded(px(6.))
        .bg(t.item_selected)
        .opacity(0.8)
        .font(theme::ui_font())
        .text_color(t.text)
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(px(6.))
                .border_1()
                .border_color(t.item_selected_ring),
        )
    }
}

/// 图标缓存键：路径、修改时间、档位任一变化都重新请求
fn icon_key(entry: &FenceEntry, bucket: u32) -> String {
    format!(
        "{}|{}|{bucket}",
        entry.path,
        entry.modified_ms.map(|m| m.to_string()).unwrap_or_default()
    )
}

/// CSS `word-break: break-all`：在每个字符之间插入零宽空格，让 GPUI 的换行器可以在任意字符处断行
pub(crate) fn break_all(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for (i, c) in text.chars().enumerate() {
        if i > 0 {
            out.push('\u{200B}');
        }
        out.push(c);
    }
    out
}

/// 带 1px 下移阴影的文字（CSS `text-shadow` 的近似：GPUI 没有文字阴影）
fn shadowed_name(text: SharedString) -> impl IntoElement {
    let text: SharedString = break_all(&text).into();
    let t = fence_tokens();
    let base = || {
        div()
            .w_full()
            .text_size(px(12.))
            .line_height(px(16.))
            .text_center()
            .line_clamp(2)
            .text_ellipsis()
    };
    div()
        .relative()
        .w_full()
        .child(
            base()
                .absolute()
                .top(px(1.))
                .left_0()
                .text_color(t.name_shadow)
                .child(text.clone()),
        )
        .child(base().relative().child(text))
}

/// 单元内容（图标框 + 名称）
fn render_cell_body(
    entry: &FenceEntry,
    icon_path: Option<&PathBuf>,
    failed: bool,
    launching: bool,
    icon_size: f32,
) -> gpui_kit::Div {
    let t = fence_tokens();
    let glyph: AnyElement = match icon_path {
        Some(path) => img(ImageSource::from(path.clone()))
            .size_full()
            .object_fit(ObjectFit::Contain)
            .with_fallback({
                let is_dir = entry.is_dir;
                move || placeholder_icon(is_dir, icon_size).into_any_element()
            })
            .into_any_element(),
        None if failed => placeholder_icon(entry.is_dir, icon_size).into_any_element(),
        None => div().into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .child(
            div()
                .relative()
                .flex_none()
                .size(px(icon_size))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(launching, |this| this.opacity(0.45))
                        .child(glyph),
                )
                .when(launching, |this| {
                    this.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(spinning(
                                SharedString::from(format!("launch-{}", entry.file_name)),
                                icon(EpIcon::Loading, icon_size * 0.45, t.text),
                            )),
                    )
                }),
        )
        .child(shadowed_name(entry.display_name.clone().into()))
}

/// 图标加载失败的占位：文件夹 `Folder`、其他 `Document`，字号 = 图标 × 0.72
fn placeholder_icon(is_dir: bool, icon_size: f32) -> impl IntoElement {
    icon(
        if is_dir {
            EpIcon::Folder
        } else {
            EpIcon::Document
        },
        icon_size * 0.72,
        fence_tokens().placeholder_icon,
    )
}

/// 移动类操作的逐项结果提示（旧 `reportResult`），返回是否已移走
fn report_result(
    result: &ItemOpResult,
    action: &str,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> bool {
    match result.status {
        ItemOpStatus::Moved => {
            if let Some(note) = result.error.as_deref().filter(|e| !e.is_empty()) {
                message::warning(window, cx, note.to_string());
            }
            true
        }
        ItemOpStatus::Cancelled => {
            message::info(window, cx, format!("已取消{action}"));
            false
        }
        ItemOpStatus::Failed => {
            message::error_with(
                window,
                cx,
                &format!("{action}失败"),
                result.error.as_deref().unwrap_or(""),
            );
            false
        }
        ItemOpStatus::Skipped => false,
    }
}

impl FenceWindow {
    /// 渲染用的列表（拖动排序期间是本地顺序）
    pub(super) fn display_items(&self) -> &[FenceEntry] {
        self.sorting.as_deref().unwrap_or(&self.items)
    }

    fn icon_size(&self) -> i32 {
        self.fence
            .as_ref()
            .map(|f| f.icon_size)
            .unwrap_or(48)
            .clamp(
                mini_todo_core::fence::MIN_ICON_SIZE,
                mini_todo_core::fence::MAX_ICON_SIZE,
            )
    }

    /// 列表变化后：选中项 / 右键菜单对应项已不在列表 → 取消
    pub(super) fn on_items_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = &self.selected
            && !self.items.iter().any(|i| &i.file_name == selected)
        {
            self.selected = None;
        }
        let menu_gone = self
            .item_menu
            .as_ref()
            .is_some_and(|m| !self.items.iter().any(|i| i.file_name == m.item.file_name));
        if menu_gone {
            self.item_menu = None;
            self.reasons.remove(&ExpandReason::ItemMenu);
        }
        cx.notify();
    }

    // ------------------------------------------------------------------
    // 图标懒加载
    // ------------------------------------------------------------------

    /// 可见（含上下 64px 外扩）的项目下标范围：按单元最矮 / 最高的行高估一个超集
    fn visible_range(&self, count: usize) -> std::ops::Range<usize> {
        let size = self.icon_size() as f32;
        let cell_w = size + 40.;
        let viewport = self.grid_scroll.bounds();
        let width = f32::from(viewport.size.width);
        let height = f32::from(viewport.size.height);
        if width <= 0. || height <= 0. {
            // 还没布局：先请求前三行
            return 0..count.min(12);
        }
        let cols = ((width / cell_w).floor() as usize).max(1);
        let short_row = 6. + size + 4. + 16. + 4. + 2.;
        let tall_row = short_row + 16.;
        let top = (-f32::from(self.grid_scroll.offset().y) - timing::ICON_PRELOAD_MARGIN).max(0.);
        let bottom = -f32::from(self.grid_scroll.offset().y) + height + timing::ICON_PRELOAD_MARGIN;
        let first_row = (top / tall_row).floor() as usize;
        let last_row = (bottom / short_row).ceil() as usize;
        (first_row * cols).min(count)..((last_row + 1) * cols).min(count)
    }

    /// 给可见且没请求过的项排队请求图标（同时最多 3 个；同一键只请求一次，失败也不重试）
    pub(super) fn pump_icons(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fence.is_none() || !self.view_expanded() {
            return;
        }
        let dpr = f64::from(window.scale_factor());
        let bucket = logic::icon_bucket(logic::icon_request_px(f64::from(self.icon_size()), dpr));
        let range = self.visible_range(self.display_items().len());
        let wanted: Vec<IconRequest> = self.display_items()[range]
            .iter()
            .map(|entry| IconRequest {
                key: icon_key(entry, bucket),
                file_name: entry.file_name.clone(),
                size_px: bucket,
            })
            .filter(|req| !self.icons.contains_key(&req.key))
            .collect();
        for req in wanted {
            self.icons.insert(req.key.clone(), IconState::Loading);
            self.icon_queue.push_back(req);
        }
        while self.icons_in_flight < timing::MAX_ICON_REQUESTS {
            let Some(req) = self.icon_queue.pop_front() else {
                break;
            };
            self.icons_in_flight += 1;
            let id = self.fence_id;
            let file_name = req.file_name.clone();
            let size = req.size_px;
            let task = spawn_db(cx, move |db| items::icon(db, id, &file_name, size));
            let key = req.key;
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.icons_in_flight = this.icons_in_flight.saturating_sub(1);
                    let state = match result {
                        Ok(path) => IconState::Ready(path),
                        Err(e) => {
                            log::debug!("[fence] 图标提取失败（{key}）: {e}");
                            IconState::Failed
                        }
                    };
                    this.icons.insert(key, state);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn icon_state(&self, entry: &FenceEntry, bucket: u32) -> Option<&IconState> {
        self.icons.get(&icon_key(entry, bucket))
    }

    // ------------------------------------------------------------------
    // 选择 / 打开 / 右键菜单
    // ------------------------------------------------------------------

    fn open_item(&mut self, entry: FenceEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.close_item_menu(window, cx);
        self.selected = Some(entry.file_name.clone());
        if !self.launching.insert(entry.file_name.clone()) {
            return;
        }
        cx.notify();
        let id = self.fence_id;
        let file_name = entry.file_name.clone();
        let task = spawn_db(cx, move |db| items::open(db, id, &file_name));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.launching.remove(&entry.file_name);
                if let Err(e) = result {
                    message::error_with(
                        window,
                        cx,
                        &format!("打开「{}」失败", entry.display_name),
                        &e,
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_item_menu(
        &mut self,
        entry: FenceEntry,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some(entry.file_name.clone());
        cx.notify();
        let task = runtime::other_fences(cx, self.fence_id);
        cx.spawn_in(window, async move |this, cx| {
            let targets = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.item_menu = Some(ItemMenu {
                    position,
                    item: entry,
                    page: MenuPage::Main,
                    targets,
                });
                this.set_reason(ExpandReason::ItemMenu, true, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn close_item_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.item_menu.take().is_some() {
            self.set_reason(ExpandReason::ItemMenu, false, window, cx);
            cx.notify();
        }
    }

    fn reveal_item(&mut self, entry: FenceEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.close_item_menu(window, cx);
        let id = self.fence_id;
        let task = spawn_db(cx, move |db| items::reveal(db, id, &entry.file_name));
        cx.spawn_in(window, async move |_, cx| {
            if let Err(e) = task.await {
                let _ =
                    cx.update(|window, cx| message::error_with(window, cx, "打开所在位置失败", &e));
            }
        })
        .detach();
    }

    /// 从本地列表去掉（不等目录监听事件），再补一次列举
    fn drop_locally(&mut self, file_name: &str, cx: &mut Context<Self>) {
        let list: Vec<FenceEntry> = self
            .items
            .iter()
            .filter(|i| i.file_name != file_name)
            .cloned()
            .collect();
        self.set_items(list, cx);
        self.refresh_items(cx);
    }

    fn restore_item(&mut self, entry: FenceEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.close_item_menu(window, cx);
        let id = self.fence_id;
        let owner = platform::hwnd(window).map(|h| h.0);
        let file_name = entry.file_name.clone();
        let task = spawn_db(cx, move |db| items::restore(db, id, &file_name, owner));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(result) => {
                    if report_result(&result, "移回桌面", window, cx) {
                        let has_note = result.error.as_deref().is_some_and(|e| !e.is_empty());
                        if !has_note && let Some(dest) = &result.dest_path {
                            message::success(
                                window,
                                cx,
                                format!("已移回 {}", logic::parent_dir(dest)),
                            );
                        }
                        this.drop_locally(&entry.file_name, cx);
                    }
                }
                Err(e) => message::error_with(window, cx, "移回桌面失败", &e),
            });
        })
        .detach();
    }

    fn move_item_to(
        &mut self,
        entry: FenceEntry,
        target: Fence,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_item_menu(window, cx);
        let id = self.fence_id;
        let owner = platform::hwnd(window).map(|h| h.0);
        let file_name = entry.file_name.clone();
        let target_id = target.id;
        let task = spawn_db(cx, move |db| {
            items::move_to(db, id, &file_name, target_id, owner)
        });
        let action = format!("移动到「{}」", target.title);
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((result, target_refresh)) => {
                    if report_result(&result, &action, window, cx) {
                        this.drop_locally(&entry.file_name, cx);
                    }
                    if let Some(refresh) = target_refresh {
                        runtime::deliver_refresh(cx, refresh);
                    }
                }
                Err(e) => message::error_with(window, cx, "移动失败", &e),
            });
        })
        .detach();
    }

    fn set_menu_page(&mut self, page: MenuPage, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.item_menu {
            menu.page = page;
            cx.notify();
        }
    }

    // ------------------------------------------------------------------
    // 箱内拖动排序
    // ------------------------------------------------------------------

    fn begin_sort(&mut self, file_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_item_menu(window, cx);
        if self.sorting.is_none() {
            self.sorting = Some(self.items.clone());
        }
        self.dragging_item = Some(file_name.to_string());
        cx.notify();
    }

    /// 拖到另一项上方：把拖动项挪到该位置（实时让位）
    fn sort_over(&mut self, target_index: usize, cx: &mut Context<Self>) {
        let Some(dragging) = self.dragging_item.clone() else {
            return;
        };
        let Some(list) = &mut self.sorting else {
            return;
        };
        let Some(from) = list.iter().position(|i| i.file_name == dragging) else {
            return;
        };
        if from == target_index || target_index >= list.len() {
            return;
        }
        let entry = list.remove(from);
        list.insert(target_index, entry);
        cx.notify();
    }

    /// 松手：顺序变了就采用并保存（失败提示「保存排序失败」），然后补一次列举（期间被跳过的推送）
    pub(super) fn finish_sort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dragging_item = None;
        let Some(list) = self.sorting.take() else {
            return;
        };
        let names: Vec<String> = list.iter().map(|i| i.file_name.clone()).collect();
        let unchanged = names.iter().eq(self.items.iter().map(|i| &i.file_name));
        cx.notify();
        if unchanged {
            return;
        }
        self.items = list;
        self.items_seq += 1;
        let id = self.fence_id;
        let task = spawn_db(cx, move |db| items::reorder(db, id, &names));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(e) = result {
                    message::error_with(window, cx, "保存排序失败", &e);
                }
                this.refresh_items(cx);
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // Ctrl+滚轮
    // ------------------------------------------------------------------

    fn on_grid_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.modifiers.control {
            // 普通滚动照常滚动，并关闭右键菜单
            self.close_item_menu(window, cx);
            cx.notify();
            return;
        }
        cx.stop_propagation();
        let now = Instant::now();
        if self
            .last_wheel_at
            .is_some_and(|at| now.duration_since(at) < timing::WHEEL_STEP_INTERVAL)
        {
            return;
        }
        self.last_wheel_at = Some(now);
        let dy = match event.delta {
            ScrollDelta::Lines(p) => f64::from(p.y),
            ScrollDelta::Pixels(p) => f64::from(f32::from(p.y)),
        };
        // GPUI 的滚轮增量向上为正（与 DOM `deltaY` 相反）
        let next = logic::next_icon_size(self.icon_size(), -dy);
        if next != self.icon_size() {
            self.set_icon_size(next, window, cx);
        }
    }

    /// 立即重排，400ms 防抖落库（不采用返回值：期间可能又滚了几格）
    fn set_icon_size(&mut self, size: i32, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(fence) = &mut self.fence {
            fence.icon_size = size;
        }
        self.icon_size_pending = true;
        self.icon_save_epoch += 1;
        let epoch = self.icon_save_epoch;
        cx.notify();
        let id = self.fence_id;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(timing::ICON_SIZE_SAVE_DELAY)
                .await;
            let Ok(task) = this.update(cx, |this, cx| {
                if this.icon_save_epoch != epoch {
                    return None;
                }
                this.icon_size_pending = false;
                let patch = mini_todo_core::db::UpdateFenceRequest {
                    icon_size: Some(size),
                    ..Default::default()
                };
                Some(spawn_db(cx, move |db| {
                    mini_todo_core::fence::update(db, id, &patch).map(|_| ())
                }))
            }) else {
                return;
            };
            if let Some(task) = task
                && let Err(e) = task.await
            {
                let _ =
                    cx.update(|window, cx| message::error_with(window, cx, "保存图标大小失败", &e));
            }
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // 渲染
    // ------------------------------------------------------------------

    pub(super) fn render_grid(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = fence_tokens();
        let size = self.icon_size() as f32;
        let dpr = f64::from(window.scale_factor());
        let bucket = logic::icon_bucket(logic::icon_request_px(f64::from(size), dpr));
        let entries: Vec<FenceEntry> = self.display_items().to_vec();
        // 右键菜单打开时不挂项目 tooltip：菜单就开在指针处，gpui 的 tooltip 不看遮挡，会盖在菜单上
        let menu_open = self.item_menu.is_some();
        let cells: Vec<AnyElement> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let selected = self.selected.as_deref() == Some(entry.file_name.as_str());
                let launching = self.launching.contains(&entry.file_name);
                let ghost = self.dragging_item.as_deref() == Some(entry.file_name.as_str());
                let (icon_path, failed) = match self.icon_state(entry, bucket) {
                    Some(IconState::Ready(path)) => (Some(path.clone()), false),
                    Some(IconState::Failed) => (None, true),
                    _ => (None, false),
                };
                let tip: SharedString = if launching {
                    format!("正在打开「{}」…", entry.display_name).into()
                } else {
                    entry.display_name.clone().into()
                };
                let down_entry = entry.clone();
                let click_entry = entry.clone();
                let menu_entry = entry.clone();
                let drag_name = entry.file_name.clone();
                let drag_entry = entry.clone();
                let drag_icon = icon_path.clone();
                let drag_failed = failed;
                let weak = cx.entity().downgrade();
                div()
                    .id(SharedString::from(format!("cell-{}", entry.file_name)))
                    .relative()
                    .w(px(size + 40.))
                    .pt(px(6.))
                    .px(px(4.))
                    .pb(px(4.))
                    .rounded(px(6.))
                    .cursor_default()
                    .when(ghost, |this| this.opacity(0.35))
                    .when(selected, |this| {
                        this.bg(t.item_selected).child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(t.item_selected_ring),
                        )
                    })
                    .when(!selected, |this| this.hover(move |s| s.bg(t.item_hover)))
                    .when(!menu_open, |this| {
                        this.tooltip(crate::ui::tooltip::native(tip))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.close_item_menu(window, cx);
                            this.selected = Some(down_entry.file_name.clone());
                            cx.notify();
                        }),
                    )
                    .on_click(
                        cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                            if event.click_count() >= 2 {
                                this.open_item(click_entry.clone(), window, cx);
                            }
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_item_menu(menu_entry.clone(), event.position, window, cx);
                        }),
                    )
                    .on_drag(
                        FenceItemDrag {
                            file_name: drag_name,
                        },
                        move |drag: &FenceItemDrag, _, window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.begin_sort(&drag.file_name, window, cx)
                            });
                            let entry = drag_entry.clone();
                            let icon = drag_icon.clone();
                            cx.new(|_| FenceItemDragView {
                                entry,
                                icon,
                                failed: drag_failed,
                                icon_size: size,
                            })
                        },
                    )
                    .on_drag_move::<FenceItemDrag>(cx.listener(
                        move |this, event: &DragMoveEvent<FenceItemDrag>, _, cx| {
                            if event.bounds.contains(&event.event.position) {
                                this.sort_over(index, cx);
                            }
                        },
                    ))
                    .on_drop::<FenceItemDrag>(cx.listener(|this, _: &FenceItemDrag, window, cx| {
                        this.finish_sort(window, cx)
                    }))
                    .child(render_cell_body(
                        entry,
                        icon_path.as_ref(),
                        failed,
                        launching,
                        size,
                    ))
                    .into_any_element()
            })
            .collect();

        div()
            .id("fence-grid")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.grid_scroll)
            .child(
                div()
                    .id("fence-grid-content")
                    .min_h_full()
                    .flex()
                    .flex_wrap()
                    .content_start()
                    .gap_y(px(2.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            // 点空白处取消选中
                            this.close_item_menu(window, cx);
                            if this.selected.take().is_some() {
                                cx.notify();
                            }
                        }),
                    )
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                        this.on_grid_wheel(event, window, cx)
                    }))
                    .on_drop::<FenceItemDrag>(cx.listener(|this, _: &FenceItemDrag, window, cx| {
                        this.finish_sort(window, cx)
                    }))
                    .children(cells),
            )
    }

    pub(super) fn render_item_menu(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.item_menu.clone()?;
        let t = fence_tokens();
        let row = |id: SharedString| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .h(px(28.))
                .px(px(8.))
                .text_size(px(12.))
                .text_color(t.menu_text)
                .whitespace_nowrap()
                .rounded(theme::radius::SM)
        };
        let entry = menu.item.clone();
        let has_targets = !menu.targets.is_empty();
        let items: Vec<AnyElement> = match menu.page {
            MenuPage::Main => {
                let (open_entry, reveal_entry, restore_entry) =
                    (entry.clone(), entry.clone(), entry.clone());
                vec![
                    row("menu-open".into())
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.menu_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_item(open_entry.clone(), window, cx)
                        }))
                        .child(icon(EpIcon::TopRight, 12., t.menu_text))
                        .child("打开")
                        .into_any_element(),
                    row("menu-reveal".into())
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.menu_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reveal_item(reveal_entry.clone(), window, cx)
                        }))
                        .child(icon(EpIcon::FolderOpened, 12., t.menu_text))
                        .child("打开所在位置")
                        .into_any_element(),
                    row("menu-restore".into())
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.menu_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.restore_item(restore_entry.clone(), window, cx)
                        }))
                        .child(icon(EpIcon::Back, 12., t.menu_text))
                        .child("移回桌面")
                        .into_any_element(),
                    row("menu-move".into())
                        .when(has_targets, |this| {
                            this.cursor_pointer()
                                .hover(move |s| s.bg(t.menu_hover))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_menu_page(MenuPage::Move, cx)
                                }))
                        })
                        .when(!has_targets, |this| {
                            this.opacity(0.5)
                                .tooltip(crate::ui::tooltip::native("没有其他收纳箱"))
                        })
                        .child(icon(EpIcon::Rank, 12., t.menu_text))
                        .child("移动到…")
                        .child(div().ml_auto().flex().items_center().map(|this| {
                            if has_targets {
                                this.child(icon(EpIcon::Right, 12., t.menu_text))
                            } else {
                                this.text_size(px(11.))
                                    .text_color(t.menu_note)
                                    .child("没有其他收纳箱")
                            }
                        }))
                        .into_any_element(),
                ]
            }
            MenuPage::Move => {
                let mut rows = vec![
                    row("menu-back".into())
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.menu_hover))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.set_menu_page(MenuPage::Main, cx)),
                        )
                        .child(icon(EpIcon::ArrowLeft, 12., t.menu_text))
                        .child("移动到")
                        .into_any_element(),
                    div()
                        .h(px(1.))
                        .mx(px(4.))
                        .my(px(3.))
                        .bg(t.menu_divider)
                        .into_any_element(),
                ];
                rows.extend(menu.targets.iter().map(|target| {
                    let target = target.clone();
                    let move_entry = entry.clone();
                    let title: SharedString = target.title.clone().into();
                    row(SharedString::from(format!("menu-target-{}", target.id)))
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.menu_hover))
                        .tooltip(crate::ui::tooltip::native(title.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.move_item_to(move_entry.clone(), target.clone(), window, cx)
                        }))
                        .child(div().min_w_0().truncate().child(title))
                        .into_any_element()
                }));
                rows
            }
        };
        let panel =
            div()
                .id("fence-item-menu")
                .occlude()
                .flex()
                .flex_col()
                .min_w(px(150.))
                .p(px(4.))
                .bg(t.menu_bg)
                .border_1()
                .border_color(t.menu_border)
                .rounded(px(6.))
                .shadow(vec![theme::shadow(0., 6., 18., 0., t.menu_shadow)])
                .font(theme::ui_font())
                .line_height(gpui_kit::relative(theme::LINE_HEIGHT))
                .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.close_item_menu(window, cx)
                }))
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .children(items);
        Some(
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Window)
                    .position(menu.position)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(4.))
                    .child(panel),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}
