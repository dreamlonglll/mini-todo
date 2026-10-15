//! 待办行（旧 `TodoItem.vue`，规格 §6）：颜色圆点（拖拽手柄，命中区 24×24）+ 标题 + 元信息行（子任务计数 /
//! 重复徽标 / 提醒时间）+ 子任务列表（展开 / 收起动画）+ 悬停操作条（完成 / 置顶 / 删除）。列表与四象限共用。

use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Bounds, Context, CursorStyle,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point, Render,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, ease_in_out,
    prelude::FluentBuilder as _, px, relative,
};

use super::{
    DragKind, DraggedTodo, MainWindow, RenderCtx, RowData, SUBTASK_ANIMATION, above_fab, logic,
};
use crate::assets::EpIcon;
use crate::state;
use crate::theme::{self, Palette};
use crate::ui::icon::icon;
use crate::ui::message_box::{self, BoxKind, ConfirmOptions};
use crate::ui::tooltip::{self, EpTooltip};

/// 行在哪个视图里
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowPlace {
    List,
    /// 四象限（象限下标 0..4；是否本象限最后一项——最后一项没有下边距）
    Quadrant {
        index: usize,
        last: bool,
    },
}

/// 颜色圆点在行内的位置：命中区 24×24 的左上角（圆点 8×8 在 (12, 14)，命中区四周外扩 8px）
const HANDLE_ORIGIN: (f32, f32) = (4., 6.);

impl MainWindow {
    pub(crate) fn render_row(
        &mut self,
        row: &RowData,
        place: RowPlace,
        rc: &RenderCtx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id;
        let palette = rc.palette;
        let colors = rc.colors;
        let dragging_any = self.drag.as_ref().is_some_and(|d| !d.dropped) || cx.has_active_drag();
        let is_placeholder = self.drag.as_ref().is_some_and(|d| d.id == id);
        let hovered = self.hovered_row == Some(id) && !dragging_any;
        let expanded = self.is_expanded(id, rc.expand_all);
        let anim = self.expand_anims.get(&id).copied();
        let show_subtasks = !row.subtasks.is_empty() && (expanded || anim.is_some());

        let row_bounds = self.row_bounds.clone();
        let measure = canvas(
            move |bounds, _, _| {
                row_bounds.borrow_mut().insert(id, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();

        let quadrant_bg = match place {
            RowPlace::Quadrant { index, .. } => {
                let (r, g, b) = logic::QUADRANTS[index].rgb;
                Some(theme::rgba_css(r, g, b, 0.1))
            }
            RowPlace::List => None,
        };
        let last_in_quadrant = matches!(place, RowPlace::Quadrant { last: true, .. });
        let hover_bg = colors.row_hover;

        let actions = hovered.then(|| {
            let bar = self.render_actions(row, &palette, rc, window, cx);
            above_fab(bar, self.fab_dimmed)
        });

        div()
            .id(("todo-row", id as u64))
            .relative()
            .flex()
            .items_start()
            .px(px(12.))
            .py(px(8.))
            .when(!last_in_quadrant, |this| this.mb(px(4.)))
            .rounded(theme::radius::BASE)
            .cursor_pointer()
            .when(!dragging_any && !palette.is_dark, |this| {
                this.hover(move |s| s.bg(hover_bg))
            })
            .when(is_placeholder, |this| match quadrant_bg {
                // 四象限里的占位：象限底色 + 半透明
                Some(bg) => this.opacity(0.5).when(!palette.is_dark, |this| this.bg(bg)),
                None => this.bg(colors.row_dragging).shadow(palette.shadow_md()),
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_row = Some(id);
                } else if this.hovered_row == Some(id) {
                    this.hovered_row = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.open_todo(id, window, cx)))
            .child(measure)
            .child(self.render_handle(row, rc, cx))
            .child(self.render_content(row, show_subtasks, expanded, anim, rc, cx))
            .children(actions)
            .into_any_element()
    }

    /// 颜色圆点 + 24×24 拖拽命中区（负边距抵消，布局仍按 8px 圆点计算）
    fn render_handle(
        &mut self,
        row: &RowData,
        rc: &RenderCtx,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = row.id;
        let weak = cx.entity().downgrade();
        let ghost_row = row.clone();
        let ghost_ctx = rc.clone();
        div()
            .id(("todo-handle", id as u64))
            .flex_none()
            .size(px(24.))
            .mt(px(-2.))
            .ml(px(-8.))
            .mr(px(4.))
            .mb(px(-8.))
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::OpenHand)
            .on_drag(DraggedTodo { id }, move |dragged, _offset, _window, cx| {
                let width = weak
                    .update(cx, |this, cx| {
                        this.begin_drag(dragged.id, cx);
                        this.row_bounds
                            .borrow()
                            .get(&dragged.id)
                            .map(|b| f32::from(b.size.width))
                    })
                    .ok()
                    .flatten()
                    .unwrap_or(300.);
                let row = ghost_row.clone();
                let rc = ghost_ctx.clone();
                cx.new(|_| DragGhost { row, rc, width })
            })
            .child(div().size(px(8.)).rounded_full().bg(row.color))
    }

    fn render_content(
        &mut self,
        row: &RowData,
        show_subtasks: bool,
        expanded: bool,
        anim: Option<super::ExpandAnim>,
        rc: &RenderCtx,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = rc.palette;
        div()
            .flex_1()
            .min_w_0()
            .font(rc.todo_font.clone())
            .child(
                div()
                    .text_size(px(rc.font_size))
                    .line_height(relative(theme::LINE_HEIGHT))
                    .text_color(palette.text_primary)
                    .when(row.completed, |this| this.line_through())
                    .child(row.title.clone()),
            )
            .when(row.has_meta(), |this| {
                this.child(self.render_meta(row, expanded, rc, cx))
            })
            .when(show_subtasks, |this| {
                let list = render_subtasks(row, rc);
                this.child(match anim {
                    Some(anim) => list
                        .with_animation(
                            (
                                "subtask-anim",
                                (row.id as u64) << 16 | (anim.generation & 0xFFFF),
                            ),
                            Animation::new(SUBTASK_ANIMATION).with_easing(ease_in_out),
                            move |el, delta| {
                                let t = if anim.expanding { delta } else { 1. - delta };
                                el.max_h(px(300. * t)).opacity(t)
                            },
                        )
                        .into_any_element(),
                    None => list.into_any_element(),
                })
            })
    }

    fn render_meta(
        &mut self,
        row: &RowData,
        expanded: bool,
        rc: &RenderCtx,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = rc.palette;
        let sub_size = rc.sub_size();
        let tertiary = palette.text_tertiary;
        let secondary = palette.text_secondary;
        let count_hover = rc.colors.subtask_count_hover;
        let id = row.id;
        let group: gpui_kit::SharedString = format!("subtask-count-{id}").into();
        let count_color = if expanded { secondary } else { tertiary };

        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .mt(px(4.))
            .text_size(px(sub_size))
            .line_height(relative(theme::LINE_HEIGHT))
            .text_color(tertiary)
            .when(!row.subtasks.is_empty(), |this| {
                let group_hover = group.clone();
                this.child(
                    div()
                        .id(("subtask-count", id as u64))
                        .group(group.clone())
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4.))
                        .px(px(4.))
                        .mx(px(-4.))
                        .rounded(theme::radius::SM)
                        .cursor_pointer()
                        .text_color(count_color)
                        .hover(move |s| s.text_color(secondary).bg(count_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_expanded(id, cx);
                        }))
                        .child(
                            icon(EpIcon::Finished, 12., count_color)
                                .group_hover(group_hover.clone(), move |s| s.text_color(secondary)),
                        )
                        .child(format!("{}/{}", row.done, row.subtasks.len()))
                        .child(
                            icon(
                                if expanded {
                                    EpIcon::ArrowDown
                                } else {
                                    EpIcon::ArrowRight
                                },
                                10.,
                                count_color,
                            )
                            .group_hover(group_hover, move |s| s.text_color(secondary)),
                        ),
                )
            })
            .when_some(row.repeat.clone(), |this, tip| {
                this.child(
                    EpTooltip::new(
                        ("repeat-tip", id as u64),
                        tip,
                        div().flex().items_center().child(icon(
                            EpIcon::RefreshRight,
                            12.,
                            theme::ep().primary,
                        )),
                    )
                    .show_after_ms(300)
                    .flex_none(),
                )
            })
            .when_some(row.notify.clone(), |this, notify| {
                this.child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap(px(4.))
                        .child(icon(EpIcon::Bell, 12., tertiary))
                        .child(wrapping_words(&notify)),
                )
            })
    }

    /// 悬停操作条：右 8 上 8，完成 / 置顶（未完成）/ 删除
    fn render_actions(
        &mut self,
        row: &RowData,
        palette: &Palette,
        rc: &RenderCtx,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id;
        let completed = row.completed;
        let title = row.title.clone();
        let actions_bounds = self.actions_bounds.clone();
        let measure = canvas(
            move |bounds: Bounds<Pixels>, _, _| actions_bounds.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        let button =
            |key: &'static str, source: EpIcon, tip: &'static str, hover_color: gpui_kit::Hsla| {
                let group: gpui_kit::SharedString = format!("todo-action-{key}-{id}").into();
                let hover_bg = palette.bg_tertiary;
                div()
                    .id((key, id as u64))
                    .group(group.clone())
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme::radius::SM)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover_bg))
                    .tooltip(tooltip::native(tip))
                    .child(
                        icon(source, 16., palette.text_secondary)
                            .group_hover(group, move |s| s.text_color(hover_color)),
                    )
            };

        div()
            .id(("todo-actions", id as u64))
            // 不 occlude：CSS 里悬停在子元素上父元素仍处于 :hover（遮挡会让行失去悬停、操作条闪烁消失）
            .absolute()
            .right(px(8.))
            .top(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .py(px(2.))
            .rounded(theme::radius::SM)
            .bg(rc.colors.actions_bg)
            .shadow(vec![theme::shadow(
                0.,
                1.,
                4.,
                0.,
                rc.colors.actions_shadow,
            )])
            .cursor_default()
            .child(measure)
            .child(
                button(
                    "todo-complete",
                    if completed {
                        EpIcon::RefreshLeft
                    } else {
                        EpIcon::Select
                    },
                    if completed { "取消完成" } else { "完成" },
                    palette.success,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    let task = this
                        .state
                        .update(cx, |state, cx| state.toggle_complete(id, cx));
                    this.notify_on_error(task, "更新待办失败", window, cx);
                })),
            )
            .when(!completed, |this| {
                this.child(
                    button("todo-top", EpIcon::Top, "置顶", palette.warning).on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            let mut ids: Vec<i64> = this
                                .state
                                .read(cx)
                                .pending_todos()
                                .iter()
                                .map(|t| t.id)
                                .filter(|t| *t != id)
                                .collect();
                            ids.insert(0, id);
                            let task = this
                                .state
                                .update(cx, |state, cx| state.reorder_todos(ids, cx));
                            this.notify_on_error(task, "保存排序失败", window, cx);
                        },
                    )),
                )
            })
            .child(
                button("todo-delete", EpIcon::Delete, "删除", palette.error).on_click(cx.listener(
                    move |_, _, window, cx| {
                        cx.stop_propagation();
                        let weak = cx.entity().downgrade();
                        message_box::confirm(
                            window,
                            cx,
                            ConfirmOptions::new(
                                "删除确认",
                                format!("确定要删除待办\"{title}\"吗？"),
                            )
                            .confirm_text("删除")
                            .cancel_text("取消")
                            .kind(BoxKind::Warning),
                            move |confirmed, window, cx| {
                                if !confirmed {
                                    return;
                                }
                                let _ = weak.update(cx, |this, cx| {
                                    let task = this
                                        .state
                                        .update(cx, |state, cx| state.delete_todo(id, cx));
                                    this.notify_on_error(task, "删除待办失败", window, cx);
                                });
                            },
                        );
                    },
                )),
            )
            .into_any_element()
    }

    // ------------------------------------------------------------------
    // 拖拽排序（列表与四象限共用的部分）
    // ------------------------------------------------------------------

    /// 开始拖拽：按当前显示顺序建会话
    pub(crate) fn begin_drag(&mut self, id: i64, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let kind = match state.view_mode {
            state::ViewMode::List => {
                let order: Vec<i64> = state.pending_todos().iter().map(|t| t.id).collect();
                DragKind::List {
                    original: order.clone(),
                    order,
                }
            }
            state::ViewMode::Quadrant => {
                let groups = state.todos_by_quadrant();
                let lists: [Vec<i64>; 4] =
                    std::array::from_fn(|i| groups[i].iter().map(|t| t.id).collect());
                let origin = lists.iter().position(|l| l.contains(&id)).unwrap_or(0);
                DragKind::Quadrant {
                    original: lists.clone(),
                    lists,
                    origin,
                    current: origin,
                }
            }
        };
        self.hovered_row = None;
        self.drag = Some(super::DragSession {
            id,
            kind,
            dropped: false,
            pointer: None,
        });
        self.start_auto_scroll(cx);
        cx.notify();
    }

    /// 拖到滚动区上下边缘 30px 内时自动滚动（SortableJS 默认 scrollSensitivity 30、scrollSpeed 10）
    fn start_auto_scroll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let keep = this.update(cx, |this, cx| {
                    let Some(session) = this.drag.as_ref().filter(|d| !d.dropped) else {
                        return false;
                    };
                    let Some(pointer) = session.pointer else {
                        return true;
                    };
                    let bounds = this.scroll.bounds();
                    let max = this.scroll.max_offset().y;
                    if bounds.size.height <= px(0.) || max <= px(0.) {
                        return true;
                    }
                    let offset = this.scroll.offset();
                    let mut next = offset.y;
                    if pointer.y < bounds.top() + px(30.) {
                        next += px(10.);
                    } else if pointer.y > bounds.bottom() - px(30.) {
                        next -= px(10.);
                    }
                    let next = next.clamp(-max, px(0.));
                    if next != offset.y {
                        this.scroll.set_offset(Point::new(offset.x, next));
                        cx.notify();
                    }
                    true
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// 拖拽中的指针位置（行内命中区按上一帧量到的各行位置计算插入点）
    pub(crate) fn record_drag_pointer(
        &mut self,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.drag.as_mut() {
            session.pointer = Some(pointer);
        }
        cx.set_active_drag_cursor_style(CursorStyle::ClosedHand, window);
    }

    /// `ids`（当前显示顺序）里除被拖项以外各行的垂直中点
    pub(crate) fn mids_without(&self, ids: &[i64], dragged: i64) -> Vec<f32> {
        let bounds = self.row_bounds.borrow();
        ids.iter()
            .filter(|id| **id != dragged)
            .filter_map(|id| bounds.get(id))
            .map(|b| f32::from(b.center().y))
            .collect()
    }

    /// 松手 / 取消：落库（列表：整体顺序；四象限：改象限 + 合并顺序），结束后按 store 重建
    pub(crate) fn finish_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.drag.as_mut() else {
            return;
        };
        if session.dropped {
            return;
        }
        session.dropped = true;
        let id = session.id;
        let handle = window.window_handle();
        match &session.kind {
            DragKind::List { original, order } => {
                if order == original {
                    self.drag = None;
                    cx.notify();
                    return;
                }
                let ids = order.clone();
                let task = self
                    .state
                    .update(cx, |state, cx| state.reorder_todos(ids, cx));
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Err(e) = result {
                            log::error!("[main] 保存排序失败: {e}");
                            crate::ui::message::show_in(
                                handle,
                                cx,
                                crate::ui::message::MessageKind::Error,
                                crate::ui::message::error_text("保存排序失败", &e),
                            );
                            // 失败：本地顺序回到库里的顺序
                            this.state.update(cx, |state, cx| state.reload_todos(cx));
                        }
                        this.drag = None;
                        cx.notify();
                    });
                })
                .detach();
            }
            DragKind::Quadrant {
                original,
                lists,
                origin,
                current,
            } => {
                let (origin, current) = (*origin, *current);
                if current == origin && lists[current] == original[current] {
                    self.drag = None;
                    cx.notify();
                    return;
                }
                // 两个顺序都在第一个 await 之前取快照
                let quadrant_ids = lists[current].clone();
                let state = self.state.read(cx);
                let global: Vec<i64> = state.pending_todos().iter().map(|t| t.id).collect();
                let todo = state.todo(id).cloned();
                let change = (current != origin)
                    .then_some(todo.as_ref())
                    .flatten()
                    .filter(|t| logic::quadrant_index(t.quadrant) != current)
                    .map(|t| {
                        let target = (current + 1) as i32;
                        (
                            target,
                            logic::resolve_quadrant_color(&t.color, t.quadrant, target),
                        )
                    });
                let state = self.state.clone();
                cx.spawn(async move |this, cx| {
                    if let Some((quadrant, color)) = change {
                        let task = state.update(cx, |state, cx| {
                            state.update_todo_quadrant(id, quadrant, Some(color), cx)
                        });
                        if let Err(e) = task.await {
                            log::error!("[main] 更新待办失败: {e}");
                            let _ = this.update(cx, |this, cx| {
                                crate::ui::message::show_in(
                                    handle,
                                    cx,
                                    crate::ui::message::MessageKind::Error,
                                    crate::ui::message::error_text("更新待办失败", &e),
                                );
                                this.drag = None;
                                cx.notify();
                            });
                            return;
                        }
                    }
                    let merged = logic::merge_quadrant_order(&global, &quadrant_ids);
                    let task = state.update(cx, |state, cx| state.reorder_todos(merged, cx));
                    let result = task.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Err(e) = result {
                            log::error!("[main] 保存排序失败: {e}");
                            crate::ui::message::show_in(
                                handle,
                                cx,
                                crate::ui::message::MessageKind::Error,
                                crate::ui::message::error_text("保存排序失败", &e),
                            );
                            this.state.update(cx, |state, cx| state.reload_todos(cx));
                        }
                        this.drag = None;
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }

    /// 按 id 取当前 store 里的行数据（拖拽会话的顺序里可能有刚被删掉的 id）
    pub(crate) fn rows_for(&self, ids: &[i64], cx: &Context<Self>) -> Vec<RowData> {
        let state = self.state.read(cx);
        ids.iter()
            .filter_map(|id| state.todo(*id))
            .filter(|t| !t.completed)
            .map(RowData::from_todo)
            .collect()
    }
}

/// 子任务列表：左缩进 20、左边线、每行 22px（`CirclePlus` / `SuccessFilled` + 单行省略的标题）
fn render_subtasks(row: &RowData, rc: &RenderCtx) -> gpui_kit::Div {
    let palette = rc.palette;
    let sub_size = rc.sub_size();
    div()
        .mt(px(4.))
        .ml(px(20.))
        .pl(px(2.))
        .border_l_1()
        .border_color(palette.border)
        .overflow_hidden()
        .children(row.subtasks.iter().map(|sub| {
            let text = div().min_w_0().truncate().child(sub.title.clone());
            let title_el = div()
                .flex_1()
                .min_w_0()
                .flex()
                .when(sub.completed, |this| this.text_color(palette.text_tertiary))
                .child(if sub.completed {
                    strike(text, palette.text_tertiary)
                } else {
                    text
                });
            // 长标题（UTF-16 长度 ≥ 20）悬停 500ms 显示完整标题
            let title_el = if logic::utf16_len(&sub.title) >= 20 {
                EpTooltip::new(("subtask-tip", sub.id as u64), sub.title.clone(), title_el)
                    .show_after_ms(500)
                    .flex_1()
                    .min_w_0()
                    .into_any_element()
            } else {
                title_el.into_any_element()
            };
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .py(px(2.))
                .text_size(px(sub_size))
                .line_height(relative(theme::LINE_HEIGHT))
                .text_color(palette.text_secondary)
                .child(if sub.completed {
                    icon(EpIcon::SuccessFilled, 12., palette.success)
                } else {
                    icon(EpIcon::CirclePlus, 12., palette.text_tertiary)
                })
                .child(title_el)
        }))
}

/// 按空格拆成可折行的片段（`10-15 17:30` 在窄的四象限里折成两行；GPUI 的折行会把放不下的空格单独
/// 留成一行，CSS 里行尾空格是悬挂的）
fn wrapping_words(text: &str) -> gpui_kit::Div {
    div().min_w_0().flex().flex_wrap().gap_x(px(3.3)).children(
        text.split(' ')
            .filter(|w| !w.is_empty())
            .map(|w| div().child(w.to_string())),
    )
}

/// CSS `text-decoration: line-through`：GPUI 自带的删除线对中文落在基线附近（像下划线），
/// 这里在行盒正中自绘 1px 线，长度 = 文字宽度（`text` 需是按内容宽度、可收缩的元素）
pub(crate) fn strike(text: gpui_kit::Div, color: gpui_kit::Hsla) -> gpui_kit::Div {
    text.relative().child(
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(relative(0.5))
            .mt(px(-0.5))
            .h(px(1.))
            .bg(color),
    )
}

/// 拖拽时跟随鼠标的影子（SortableJS fallback：原行克隆、透明度 0.8、无背景、不处于 hover）
pub(crate) struct DragGhost {
    row: RowData,
    rc: RenderCtx,
    width: f32,
}

impl Render for DragGhost {
    fn render(
        &mut self,
        _window: &mut Window,
        _cx: &mut gpui_kit::Context<Self>,
    ) -> impl IntoElement {
        let rc = &self.rc;
        let palette = rc.palette;
        let row = &self.row;
        div()
            .ml(px(-HANDLE_ORIGIN.0))
            .mt(px(-HANDLE_ORIGIN.1))
            .w(px(self.width))
            .opacity(0.8)
            .flex()
            .items_start()
            .px(px(12.))
            .py(px(8.))
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .child(
                div()
                    .flex_none()
                    .size(px(8.))
                    .mt(px(6.))
                    .mr(px(12.))
                    .rounded_full()
                    .bg(row.color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font(rc.todo_font.clone())
                    .child(
                        div()
                            .text_size(px(rc.font_size))
                            .text_color(palette.text_primary)
                            .child(row.title.clone()),
                    )
                    .when(row.has_meta(), |this| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .mt(px(4.))
                                .text_size(px(rc.sub_size()))
                                .text_color(palette.text_tertiary)
                                .when(!row.subtasks.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .child(icon(
                                                EpIcon::Finished,
                                                12.,
                                                palette.text_tertiary,
                                            ))
                                            .child(format!("{}/{}", row.done, row.subtasks.len()))
                                            .child(icon(
                                                EpIcon::ArrowRight,
                                                10.,
                                                palette.text_tertiary,
                                            )),
                                    )
                                })
                                .when(row.repeat.is_some(), |this| {
                                    this.child(icon(EpIcon::RefreshRight, 12., theme::ep().primary))
                                })
                                .when_some(row.notify.clone(), |this, notify| {
                                    this.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .child(icon(EpIcon::Bell, 12., palette.text_tertiary))
                                            .child(notify),
                                    )
                                }),
                        )
                    }),
            )
    }
}

/// 状态里的待办 → 行数据（按给定顺序）
pub(crate) fn rows_of(todos: &[&mini_todo_core::db::Todo]) -> Vec<RowData> {
    todos.iter().map(|t| RowData::from_todo(t)).collect()
}
