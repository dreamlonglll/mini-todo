//! 待办详情 / 编辑窗口（旧 `EditorView.vue`）。规格：`research/ui-editor-windows.md` §2、§3。
//!
//! 窗口 1080×600、可缩放、不透明、无系统标题栏，在打开方所在显示器居中；打开方：主窗口（列表 / 四象限 /
//! 日历 / FAB，模态）、已完成窗口（模态）、托盘（无 owner）。左侧 700（随窗口变宽）是详情或表单，右侧固定 380 是
//! 子任务面板。
//!
//! | 形态 | 条件 | 标题 | 左侧 | 底部栏 | 子任务面板 |
//! |---|---|---|---|---|---|
//! | 只读详情 | `is_view_mode` | 待办详情 | [`view`] | 无 | 可勾选、拖拽排序、查看 |
//! | 编辑已有 | 已有 id | 编辑待办 | [`form`] | 完成任务 / 重新打开 + 取消 + 保存 | 全功能（含导入） |
//! | 新建 | 无 id | 新建待办 | [`form`]（标题自动聚焦） | 取消 + 创建 | 暂存在内存（[`DraftSubtasks`]） |
//!
//! 只读详情点 [编辑] 原地切换（不重新加载、不重拍快照），没有切回只读的入口。保存规则（只在变化时发送提醒时间、
//! 描述总是发送、新建时第二次 update 写重复设置、新建后续步骤失败时原地切成编辑该待办……）见 [`save`]。
//! 快捷键：Esc 关闭（有未保存修改时确认）、Ctrl/Cmd+Enter 保存（只读时无效），子任务窗口打开期间全部失效。

mod desc_dialog;
mod form;
mod save;
mod subtasks;
mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, ScrollHandle,
    Styled as _, Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _, relative,
};
use mini_todo_core::db::Todo;

use super::subtask::{DraftSubtasks, SubtaskMode};
use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size};
use crate::db::spawn_db;
use crate::state::DEFAULT_TODO_COLOR;
use crate::theme::{self, hex};
use crate::ui::ep_color_picker::{EpColorPicker, EpColorPickerEvent};
use crate::ui::ep_date_picker::{EpDatePicker, EpDatePickerEvent};
use crate::ui::ep_input_number::{EpInputNumber, EpInputNumberEvent};
use crate::ui::ep_popper::Anchor;
use crate::ui::ep_select::{EpSelect, EpSelectEvent, EpSelectOption};
use crate::ui::ep_time_picker::{EpTimePicker, EpTimePickerEvent};
use crate::ui::markdown_editor::{MarkdownEditor, MarkdownEditorEvent};
use crate::ui::markdown_image_viewer::ImageViewer;
use crate::ui::{message, message_box};
use crate::util::datetime::{self, DateTimeKind};
use crate::util::editor_shortcuts::{self, EditorClose, EditorSubmit};

/// 打开方式（`mode=view` 只在有 id 时生效）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorMode {
    /// 只读详情
    #[default]
    View,
    /// 编辑表单（新建总是编辑）
    Edit,
}

impl EditorMode {
    pub fn parse(value: &str) -> Option<EditorMode> {
        match value {
            "view" => Some(EditorMode::View),
            "edit" => Some(EditorMode::Edit),
            _ => None,
        }
    }
}

pub const WIDTH: f32 = 1080.;
pub const HEIGHT: f32 = 600.;
/// 右侧子任务面板宽度
pub const PANEL_WIDTH: f32 = 380.;

/// 预设色（`PRESET_COLORS`，顺序即显示顺序）
pub const PRESET_COLORS: [(&str, &str); 8] = [
    ("红色", "#EF4444"),
    ("橙色", "#F59E0B"),
    ("黄色", "#EAB308"),
    ("绿色", "#10B981"),
    ("青色", "#06B6D4"),
    ("蓝色", "#3B82F6"),
    ("紫色", "#8B5CF6"),
    ("粉色", "#EC4899"),
];

/// 象限定义（`QUADRANT_INFO`，顺序即选择器网格顺序）：(id, 名称, 颜色)
pub const QUADRANTS: [(i32, &str, u32); 4] = [
    (1, "重要且紧急", 0xEF4444),
    (2, "重要不紧急", 0xF59E0B),
    (3, "紧急不重要", 0x3B82F6),
    (4, "不紧急不重要", 0x10B981),
];

/// 象限颜色（非法象限按默认色）
pub fn quadrant_color(quadrant: i32) -> &'static str {
    match quadrant {
        1 => "#EF4444",
        2 => "#F59E0B",
        3 => "#3B82F6",
        4 => "#10B981",
        _ => DEFAULT_TODO_COLOR,
    }
}

/// `resolveQuadrantColor`：颜色还是旧象限的默认色（用户没改过）时跟随新象限，否则保留
pub fn resolve_quadrant_color(current: Option<&str>, prev: i32, next: i32) -> Option<String> {
    match current {
        Some(c) if theme::is_same_color(c, quadrant_color(prev)) => {
            Some(quadrant_color(next).to_string())
        }
        other => other.map(str::to_string),
    }
}

/// 提前提醒选项（`-1` = 自定义）
pub const NOTIFY_BEFORE_OPTIONS: [(i32, &str); 6] = [
    (0, "准时"),
    (5, "5 分钟前"),
    (15, "15 分钟前"),
    (30, "30 分钟前"),
    (60, "1 小时前"),
    (-1, "自定义"),
];

/// 重复方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatType {
    #[default]
    Daily,
    Weekly,
    Monthly,
}

impl RepeatType {
    pub fn as_str(self) -> &'static str {
        match self {
            RepeatType::Daily => "daily",
            RepeatType::Weekly => "weekly",
            RepeatType::Monthly => "monthly",
        }
    }

    pub fn parse(value: &str) -> Option<RepeatType> {
        match value {
            "daily" => Some(RepeatType::Daily),
            "weekly" => Some(RepeatType::Weekly),
            "monthly" => Some(RepeatType::Monthly),
            _ => None,
        }
    }

    /// 单位：天 / 周 / 月
    pub fn unit(self) -> &'static str {
        match self {
            RepeatType::Daily => "天",
            RepeatType::Weekly => "周",
            RepeatType::Monthly => "月",
        }
    }

    fn index(self) -> i32 {
        match self {
            RepeatType::Daily => 0,
            RepeatType::Weekly => 1,
            RepeatType::Monthly => 2,
        }
    }

    fn from_index(index: i32) -> RepeatType {
        match index {
            1 => RepeatType::Weekly,
            2 => RepeatType::Monthly,
            _ => RepeatType::Daily,
        }
    }
}

/// 星期（`周一`=1 … `周日`=7）
pub const WEEKDAYS: [(u8, &str); 7] = [
    (1, "周一"),
    (2, "周二"),
    (3, "周三"),
    (4, "周四"),
    (5, "周五"),
    (6, "周六"),
    (7, "周日"),
];

/// `"1,3,5"` → `[1, 3, 5]`（只留 1..=7）
pub fn parse_weekdays(value: Option<&str>) -> Vec<u8> {
    value
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse::<u8>().ok())
        .filter(|d| (1..=7).contains(d))
        .collect()
}

/// 编辑窗口视图
pub struct EditorWindow {
    // ---- 身份与模式 ----
    /// 可变：新建后若后续步骤失败会原地切成该 id（§3.8）
    todo_id: Option<i64>,
    is_view_mode: bool,
    /// 已加载的待办（子任务列表与完成状态的来源）
    todo: Option<Todo>,
    images_dir: Arc<PathBuf>,
    focus: FocusHandle,

    // ---- 表单 ----
    title: Entity<InputState>,
    description: Entity<MarkdownEditor>,
    color: Option<String>,
    color_picker: Entity<EpColorPicker>,
    quadrant: i32,
    notify_before: i32,
    is_custom_notify_before: bool,
    notify_before_select: Entity<EpSelect<i32>>,
    custom_notify_before: Entity<EpInputNumber>,
    notify_date: Entity<EpDatePicker>,
    notify_time: Entity<EpTimePicker>,
    start_date: Entity<EpDatePicker>,
    start_time: Entity<EpTimePicker>,
    end_date: Entity<EpDatePicker>,
    end_time: Entity<EpTimePicker>,
    repeat_enabled: bool,
    repeat_type: RepeatType,
    repeat_type_select: Entity<EpSelect<i32>>,
    repeat_interval: Entity<EpInputNumber>,
    repeat_weekdays: Vec<u8>,
    repeat_month_day: Entity<EpInputNumber>,
    original_notify_at: Option<String>,
    original_start_time: Option<String>,
    original_end_time: Option<String>,
    clean_snapshot: String,
    saving: bool,
    updating_complete: bool,

    // ---- 子任务 ----
    drafts: Entity<DraftSubtasks>,
    add_input: Entity<InputState>,
    renaming: Option<subtasks::Renaming>,
    hovered_row: Option<i64>,
    /// 每行子任务上一帧的位置（拖影对齐整行）
    row_bounds: Rc<RefCell<HashMap<i64, Bounds<Pixels>>>>,
    import_menu_open: bool,
    import_anchor: Anchor,
    /// 拖拽排序中的当前顺序（`None` = 没在拖）
    drag_order: Option<Vec<i64>>,
    dragging: Option<i64>,
    subtask_window: Option<AnyWindowHandle>,

    // ---- 浮层 ----
    desc_dialog: Option<desc_dialog::DescDialog>,
    image_viewer: Option<Entity<ImageViewer>>,
    form_scroll: ScrollHandle,
    view_scroll: ScrollHandle,
    panel_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EditorWindow {
    pub fn new(
        todo_id: Option<i64>,
        mode: EditorMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let images_dir = Arc::new(mini_todo_core::db::paths::images_dir());
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("请输入待办标题")
                .validate(|text, _| text.chars().count() <= 100)
        });
        let description = cx.new(|cx| {
            MarkdownEditor::new("editor-desc", images_dir.clone(), Some((5, 12)), window, cx)
        });
        let predefine = PRESET_COLORS.iter().map(|(_, c)| c.to_string()).collect();
        let color_picker =
            cx.new(|cx| EpColorPicker::new("editor-color", window, cx).predefine(predefine));
        let notify_before_select = cx.new(|cx| {
            EpSelect::new("editor-notify-before", window, cx).with_options(
                NOTIFY_BEFORE_OPTIONS
                    .iter()
                    .map(|(v, l)| EpSelectOption::new(*v, *l))
                    .collect(),
            )
        });
        let custom_notify_before = cx.new(|cx| {
            EpInputNumber::new("editor-custom-before", 15, 1, 1440, window, cx)
                .with_suffix("分钟前")
        });
        let notify_date =
            cx.new(|cx| EpDatePicker::new("editor-notify-date", "选择日期", window, cx));
        let notify_time = cx.new(|cx| EpTimePicker::new("editor-notify-time", "时间", window, cx));
        let start_date =
            cx.new(|cx| EpDatePicker::new("editor-start-date", "开始日期", window, cx));
        let start_time = cx.new(|cx| EpTimePicker::new("editor-start-time", "时间", window, cx));
        let end_date = cx.new(|cx| EpDatePicker::new("editor-end-date", "截止日期", window, cx));
        let end_time = cx.new(|cx| EpTimePicker::new("editor-end-time", "时间", window, cx));
        let repeat_type_select = cx.new(|cx| {
            EpSelect::new("editor-repeat-type", window, cx).with_options(vec![
                EpSelectOption::new(0, "每天"),
                EpSelectOption::new(1, "每周"),
                EpSelectOption::new(2, "每月"),
            ])
        });
        let repeat_interval =
            cx.new(|cx| EpInputNumber::new("editor-repeat-interval", 1, 1, 99, window, cx));
        let repeat_month_day =
            cx.new(|cx| EpInputNumber::new("editor-repeat-day", 1, 1, 31, window, cx));
        let drafts = cx.new(|_| DraftSubtasks::default());
        let add_input = cx.new(|cx| InputState::new(window, cx).placeholder("添加子任务..."));

        let mut subscriptions = vec![
            cx.observe(&drafts, |_, _, cx| cx.notify()),
            cx.observe_global::<WindowManager>(|this, cx| {
                // 子任务窗口关了：遮罩去掉、快捷键恢复
                if let Some(handle) = this.subtask_window
                    && WindowManager::kind_of(cx, handle.window_id()).is_none()
                {
                    this.subtask_window = None;
                }
                cx.notify();
            }),
            cx.subscribe_in(&title, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &description,
                window,
                |_, _, _: &MarkdownEditorEvent, _, cx| cx.notify(),
            ),
            cx.subscribe_in(
                &color_picker,
                window,
                |this, _, event: &EpColorPickerEvent, _, cx| {
                    let EpColorPickerEvent::Change(color) = event;
                    this.color = color.clone();
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &notify_before_select,
                window,
                |this, _, event: &EpSelectEvent<i32>, window, cx| {
                    if let EpSelectEvent::Change(Some(value)) = event {
                        this.on_notify_before_change(*value, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &custom_notify_before,
                window,
                |_, _, _: &EpInputNumberEvent, _, cx| cx.notify(),
            ),
            cx.subscribe_in(
                &repeat_type_select,
                window,
                |this, _, event: &EpSelectEvent<i32>, _, cx| {
                    if let EpSelectEvent::Change(Some(value)) = event {
                        this.repeat_type = RepeatType::from_index(*value);
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &repeat_interval,
                window,
                |_, _, _: &EpInputNumberEvent, _, cx| cx.notify(),
            ),
            cx.subscribe_in(
                &repeat_month_day,
                window,
                |_, _, _: &EpInputNumberEvent, _, cx| cx.notify(),
            ),
            cx.subscribe_in(
                &add_input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    // 只响应无修饰的 Enter（`@keydown.enter.exact`）；Ctrl+Enter 由窗口保存待办
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    } => this.add_subtask(window, cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            ),
        ];
        // 日期清空时对应的时间选择器禁用（保留旧值）
        for (date, time) in [
            (notify_date.clone(), notify_time.clone()),
            (start_date.clone(), start_time.clone()),
            (end_date.clone(), end_time.clone()),
        ] {
            let time_for_date = time.clone();
            subscriptions.push(cx.subscribe_in(
                &date,
                window,
                move |_, _, event: &EpDatePickerEvent, _, cx| {
                    let EpDatePickerEvent::Change(value) = event;
                    let disabled = value.is_none();
                    time_for_date.update(cx, |t, cx| t.set_disabled(disabled, cx));
                    cx.notify();
                },
            ));
            subscriptions.push(cx.subscribe_in(
                &time,
                window,
                |_, _, _: &EpTimePickerEvent, _, cx| cx.notify(),
            ));
        }
        for time in [&notify_time, &start_time, &end_time] {
            time.update(cx, |t, cx| t.set_disabled(true, cx));
        }

        let focus = cx.focus_handle();
        let is_view_mode = mode == EditorMode::View && todo_id.is_some();
        let mut this = Self {
            todo_id,
            is_view_mode,
            todo: None,
            images_dir,
            focus,
            title,
            description,
            color: Some(DEFAULT_TODO_COLOR.to_string()),
            color_picker,
            quadrant: 4,
            notify_before: 15,
            is_custom_notify_before: false,
            notify_before_select,
            custom_notify_before,
            notify_date,
            notify_time,
            start_date,
            start_time,
            end_date,
            end_time,
            repeat_enabled: false,
            repeat_type: RepeatType::Daily,
            repeat_type_select,
            repeat_interval,
            repeat_weekdays: Vec::new(),
            repeat_month_day,
            original_notify_at: None,
            original_start_time: None,
            original_end_time: None,
            clean_snapshot: String::new(),
            saving: false,
            updating_complete: false,
            drafts,
            add_input,
            renaming: None,
            hovered_row: None,
            row_bounds: Rc::default(),
            import_menu_open: false,
            import_anchor: Anchor::new(),
            drag_order: None,
            dragging: None,
            subtask_window: None,
            desc_dialog: None,
            image_viewer: None,
            form_scroll: ScrollHandle::new(),
            view_scroll: ScrollHandle::new(),
            panel_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.sync_controls(window, cx);
        this.install_image_click(cx);
        // 组件创建时先按空表单拍一次（加载完成前按 Esc 不会误报）
        this.clean_snapshot = this.form_snapshot(cx);

        if todo_id.is_some() {
            window.focus(&this.focus, cx);
            this.load_todo(true, window, cx);
        } else {
            this.title.update(cx, |state, cx| state.focus(window, cx));
        }
        this
    }

    /// 系统窗口标题：已有待办 `待办详情`，新建 `新建待办`
    pub fn os_title(todo_id: Option<i64>) -> &'static str {
        if todo_id.is_some() {
            "待办详情"
        } else {
            "新建待办"
        }
    }

    /// `isEdit`：是已存在的待办
    fn is_edit(&self) -> bool {
        self.todo_id.is_some()
    }

    fn header_title(&self) -> &'static str {
        if self.is_view_mode {
            "待办详情"
        } else if self.is_edit() {
            "编辑待办"
        } else {
            "新建待办"
        }
    }

    // ------------------------------------------------------------------
    // 表单派生值
    // ------------------------------------------------------------------

    fn form_notify_at(&self, cx: &App) -> Option<String> {
        datetime::compose_date_time(
            self.notify_date.read(cx).value().as_deref(),
            self.notify_time.read(cx).value().as_deref(),
            DateTimeKind::Notify,
        )
    }

    fn form_start_time(&self, cx: &App) -> Option<String> {
        datetime::compose_date_time(
            self.start_date.read(cx).value().as_deref(),
            self.start_time.read(cx).value().as_deref(),
            DateTimeKind::Start,
        )
    }

    fn form_end_time(&self, cx: &App) -> Option<String> {
        datetime::compose_date_time(
            self.end_date.read(cx).value().as_deref(),
            self.end_time.read(cx).value().as_deref(),
            DateTimeKind::End,
        )
    }

    fn custom_before(&self, cx: &App) -> i32 {
        self.custom_notify_before.read(cx).value()
    }

    fn repeat_interval_value(&self, cx: &App) -> i32 {
        self.repeat_interval.read(cx).value()
    }

    fn repeat_month_day_value(&self, cx: &App) -> i32 {
        self.repeat_month_day.read(cx).value()
    }

    /// 存储形式的描述
    fn description_value(&self, cx: &App) -> String {
        self.description.read(cx).value(cx)
    }

    /// 旧 `formSnapshot()`：与加载后的快照比较判断是否有未保存修改
    fn form_snapshot(&self, cx: &App) -> String {
        let mut weekdays = self.repeat_weekdays.clone();
        weekdays.sort_unstable();
        let pending: Vec<serde_json::Value> = self
            .drafts
            .read(cx)
            .items
            .iter()
            .map(|s| serde_json::json!([s.title, s.content, s.completed]))
            .collect();
        serde_json::json!([
            self.title.read(cx).value().to_string(),
            self.description_value(cx),
            self.color,
            self.quadrant,
            self.form_notify_at(cx),
            if self.is_custom_notify_before {
                self.custom_before(cx)
            } else {
                self.notify_before
            },
            self.form_start_time(cx),
            self.form_end_time(cx),
            self.repeat_enabled,
            self.repeat_type.as_str(),
            self.repeat_interval_value(cx),
            weekdays,
            self.repeat_month_day_value(cx),
            pending,
        ])
        .to_string()
    }

    /// 旧 `markFormClean()`
    fn mark_clean(&mut self, cx: &App) {
        self.clean_snapshot = self.form_snapshot(cx);
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.form_snapshot(cx) != self.clean_snapshot
    }

    /// 把表单值推到各控件（加载后 / 初始化）
    fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let color = self.color.clone();
        self.color_picker
            .update(cx, |picker, cx| picker.set_value(color, cx));
        let select_value = if self.is_custom_notify_before {
            -1
        } else {
            self.notify_before
        };
        self.notify_before_select
            .update(cx, |select, cx| select.set_value(Some(select_value), cx));
        let repeat_index = self.repeat_type.index();
        self.repeat_type_select
            .update(cx, |select, cx| select.set_value(Some(repeat_index), cx));
        let _ = window;
    }

    fn on_notify_before_change(
        &mut self,
        value: i32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if value == -1 {
            self.is_custom_notify_before = true;
            self.notify_before = self.custom_before(cx);
        } else {
            self.is_custom_notify_before = false;
            self.notify_before = value;
        }
        cx.notify();
    }

    /// 点象限：颜色还是旧象限默认色时跟随新象限
    fn select_quadrant(&mut self, quadrant: i32, cx: &mut Context<Self>) {
        self.color = resolve_quadrant_color(self.color.as_deref(), self.quadrant, quadrant);
        self.quadrant = quadrant;
        let color = self.color.clone();
        self.color_picker
            .update(cx, |picker, cx| picker.set_value(color, cx));
        cx.notify();
    }

    fn select_color(&mut self, color: &str, cx: &mut Context<Self>) {
        self.color = Some(color.to_string());
        let value = self.color.clone();
        self.color_picker
            .update(cx, |picker, cx| picker.set_value(value, cx));
        cx.notify();
    }

    // ------------------------------------------------------------------
    // 加载
    // ------------------------------------------------------------------

    /// 旧 `loadTodo(refreshForm)`：`refresh_form = false` 只更新 `todo`（子任务、完成态），不重建表单
    fn load_todo(&mut self, refresh_form: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.todo_id else {
            return;
        };
        let task = spawn_db(cx, move |db| mini_todo_core::todo::get(db, id));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(todo) => {
                    if refresh_form {
                        this.fill_form(&todo, window, cx);
                    }
                    this.todo = Some(todo);
                    if refresh_form {
                        // 等一帧让控件把时间规范化后再拍快照
                        cx.on_next_frame(window, |this, _, cx| this.mark_clean(cx));
                    }
                    cx.notify();
                }
                Err(e) => message::error_with(window, cx, "加载待办失败", &e),
            });
        })
        .detach();
    }

    /// 用加载的待办重建表单
    fn fill_form(&mut self, todo: &Todo, window: &mut Window, cx: &mut Context<Self>) {
        self.title
            .update(cx, |s, cx| s.set_value(todo.title.clone(), window, cx));
        let description = todo.description.clone().unwrap_or_default();
        self.description
            .update(cx, |d, cx| d.set_value(&description, window, cx));
        self.color = Some(todo.color.clone());
        self.quadrant = todo.quadrant;
        self.notify_before = todo.notify_before;
        self.original_notify_at = todo.notify_at.clone();
        self.original_start_time = todo.start_time.clone();
        self.original_end_time = todo.end_time.clone();

        for (value, kind, date, time) in [
            (
                todo.notify_at.clone(),
                DateTimeKind::Notify,
                self.notify_date.clone(),
                self.notify_time.clone(),
            ),
            (
                todo.start_time.clone(),
                DateTimeKind::Start,
                self.start_date.clone(),
                self.start_time.clone(),
            ),
            (
                todo.end_time.clone(),
                DateTimeKind::End,
                self.end_date.clone(),
                self.end_time.clone(),
            ),
        ] {
            let parts = datetime::split_date_time(value.as_deref(), kind);
            let (d, t) = match parts {
                Some(p) => (Some(p.date), Some(p.time)),
                None => (None, None),
            };
            date.update(cx, |p, cx| p.set_value(d.as_deref(), window, cx));
            time.update(cx, |p, cx| {
                p.set_value(t.as_deref(), window, cx);
                p.set_disabled(d.is_none(), cx);
            });
        }

        if ![0, 5, 15, 30, 60].contains(&todo.notify_before) {
            self.is_custom_notify_before = true;
            let v = todo.notify_before;
            self.custom_notify_before
                .update(cx, |n, cx| n.set_value(v, window, cx));
        } else {
            self.is_custom_notify_before = false;
        }

        self.repeat_enabled = todo.repeat_enabled;
        if let Some(t) = todo.repeat_type.as_deref().and_then(RepeatType::parse) {
            self.repeat_type = t;
        }
        let interval = if todo.repeat_interval > 0 {
            todo.repeat_interval
        } else {
            1
        };
        self.repeat_interval
            .update(cx, |n, cx| n.set_value(interval, window, cx));
        self.repeat_weekdays = parse_weekdays(todo.repeat_weekdays.as_deref());
        let day = todo.repeat_month_day.filter(|d| *d > 0).unwrap_or(1);
        self.repeat_month_day
            .update(cx, |n, cx| n.set_value(day, window, cx));
        self.sync_controls(window, cx);
    }

    /// 旧 `enterEditState`：新建 / 保存成功后视为"编辑该待办"
    fn enter_edit_state(&mut self, todo: Todo) {
        self.todo_id = Some(todo.id);
        self.original_notify_at = todo.notify_at.clone();
        self.original_start_time = todo.start_time.clone();
        self.original_end_time = todo.end_time.clone();
        self.todo = Some(todo);
    }

    // ------------------------------------------------------------------
    // 关闭 / 快捷键
    // ------------------------------------------------------------------

    fn close_window(window: &mut Window) {
        window.remove_window();
    }

    /// 子任务窗口打开期间窗口级快捷键全部失效
    fn shortcuts_enabled(&self, cx: &App) -> bool {
        self.subtask_window
            .is_none_or(|h| WindowManager::kind_of(cx, h.window_id()).is_none())
    }

    /// 有弹出层（下拉、日期 / 时间 / 颜色面板、导入菜单）就先关掉它，返回是否关了
    fn close_popups(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut closed = false;
        if self.import_menu_open {
            self.import_menu_open = false;
            closed = true;
        }
        for picker in [&self.notify_date, &self.start_date, &self.end_date] {
            if picker.read(cx).is_open() {
                picker.update(cx, |p, cx| p.close(cx));
                closed = true;
            }
        }
        for picker in [&self.notify_time, &self.start_time, &self.end_time] {
            if picker.read(cx).is_open() {
                picker.update(cx, |p, cx| p.close(window, cx));
                closed = true;
            }
        }
        for select in [&self.notify_before_select, &self.repeat_type_select] {
            if select.read(cx).is_open() {
                select.update(cx, |s, cx| s.close(cx));
                closed = true;
            }
        }
        if self.color_picker.read(cx).is_open() {
            self.color_picker.update(cx, |p, cx| p.close(cx));
            closed = true;
        }
        if closed {
            cx.notify();
        }
        closed
    }

    fn any_popup_open(&self, cx: &App) -> bool {
        self.import_menu_open
            || self.notify_date.read(cx).is_open()
            || self.start_date.read(cx).is_open()
            || self.end_date.read(cx).is_open()
            || self.notify_time.read(cx).is_open()
            || self.start_time.read(cx).is_open()
            || self.end_time.read(cx).is_open()
            || self.notify_before_select.read(cx).is_open()
            || self.repeat_type_select.read(cx).is_open()
            || self.color_picker.read(cx).is_open()
    }

    fn on_close_action(&mut self, _: &EditorClose, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shortcuts_enabled(cx) {
            return;
        }
        if message_box::is_open(window, cx) {
            return;
        }
        if let Some(viewer) = &self.image_viewer {
            viewer.update(cx, |v, cx| v.close(cx));
            self.image_viewer = None;
            cx.notify();
            return;
        }
        if self.desc_dialog.is_some() {
            self.close_desc_dialog(false, window, cx);
            return;
        }
        if self.renaming.is_some() {
            // 只取消行内改名，不关窗口；焦点还给窗口
            self.renaming = None;
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if self.close_popups(window, cx) {
            return;
        }
        self.close_with_confirm(window, cx);
    }

    fn on_submit_action(&mut self, _: &EditorSubmit, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shortcuts_enabled(cx)
            || self.is_view_mode
            || self.desc_dialog.is_some()
            || self.image_viewer.is_some()
            || message_box::is_open(window, cx)
            || self.any_popup_open(cx)
        {
            return;
        }
        self.handle_save(window, cx);
    }

    /// 旧 `closeWithConfirm()`（只有 Esc 走这里；标题栏 X、取消直接关）
    fn close_with_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.updating_complete {
            return;
        }
        if !self.is_view_mode && self.is_dirty(cx) {
            message_box::confirm(
                window,
                cx,
                message_box::ConfirmOptions::new("放弃修改", "有未保存的修改，确定放弃并关闭吗？")
                    .confirm_text("放弃")
                    .cancel_text("继续编辑")
                    .kind(message_box::BoxKind::Warning),
                |confirmed, window, _| {
                    if confirmed {
                        Self::close_window(window);
                    }
                },
            );
            return;
        }
        Self::close_window(window);
    }

    /// 图片预览（描述渲染里点图片）
    fn install_image_click(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let handler: crate::ui::markdown::ImageClickHandler =
            std::rc::Rc::new(move |paths, index, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.open_image_viewer(paths, index, window, cx)
                });
            });
        self.description
            .update(cx, |d, _| d.set_on_image_click(handler));
    }

    fn open_image_viewer(
        &mut self,
        paths: Vec<PathBuf>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let viewer = cx.new(|cx| ImageViewer::new(paths, index, window, cx));
        self._subscriptions
            .push(cx.observe(&viewer, |this, viewer, cx| {
                if viewer.read(cx).is_closed() {
                    this.image_viewer = None;
                    cx.notify();
                }
            }));
        self.image_viewer = Some(viewer);
        cx.notify();
    }

    fn open_subtask_window(
        &mut self,
        subtask_id: i64,
        mode: SubtaskMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.shortcuts_enabled(cx) {
            return;
        }
        let parent = ChildOf::window(window, cx);
        let handle = if self.is_edit() {
            super::subtask::open(cx, Some(parent), subtask_id, mode)
        } else {
            if !self
                .drafts
                .read(cx)
                .items
                .iter()
                .any(|s| s.id == subtask_id)
            {
                return;
            }
            super::subtask::open_draft(cx, Some(parent), self.drafts.clone(), subtask_id, mode)
        };
        let Some(handle) = handle else {
            message::error(window, cx, "打开子任务窗口失败");
            return;
        };
        self.subtask_window = Some(handle);
        // 编辑模式关闭后只刷新当前待办（不重建表单）
        if mode == SubtaskMode::Edit && self.is_edit() {
            let weak = cx.entity().downgrade();
            let editor_window = window.window_handle();
            WindowManager::on_closed(cx, handle, move |cx| {
                let _ = editor_window.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.load_todo(false, window, cx));
                });
            });
        }
        cx.notify();
    }
}

impl gpui_kit::Focusable for EditorWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for EditorWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 拖拽排序结束（包括拖到窗口外放下）：提交当前顺序
        if self.drag_order.is_some() && !cx.has_active_drag() {
            cx.defer_in(window, |this, window, cx| this.finish_drag(window, cx));
        }
        let palette = theme::light();
        let left = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_header(cx))
            .child(if self.is_view_mode {
                self.render_view(cx).into_any_element()
            } else {
                self.render_form(window, cx).into_any_element()
            })
            .when(!self.is_view_mode, |this| {
                this.child(self.render_footer(cx))
            });
        let viewer = self.image_viewer.clone();
        let dialog = self
            .desc_dialog
            .is_some()
            .then(|| self.render_desc_dialog(window, cx).into_any_element());
        let import_menu = self.render_import_menu(window, cx);
        div()
            .id("editor-window")
            .key_context(editor_shortcuts::EDITOR_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_close_action))
            .on_action(cx.listener(Self::on_submit_action))
            .relative()
            .size_full()
            .flex()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .text_color(palette.text_primary)
            .bg(hex(0xFFFFFF))
            .child(left)
            .child(self.render_panel(window, cx))
            .children(import_menu)
            .children(super::modal_overlay(window, cx, 0.15))
            .children(dialog)
            .children(viewer.map(|v| div().absolute().inset_0().child(v)))
    }
}

/// 打开编辑窗口。`parent`：打开方（主窗口 `ChildOf::window(window, cx)`；托盘 `ChildOf::handle(main, cx).unowned()`；
/// 截图调试 `None`）。`todo_id = None` 为新建。打开方已有模态子窗口时只把它带到前台、返回 `None`。
pub fn open(
    cx: &mut App,
    parent: Option<ChildOf>,
    todo_id: Option<i64>,
    mode: EditorMode,
) -> Option<AnyWindowHandle> {
    editor_shortcuts::init(cx);
    let mode = if todo_id.is_none() {
        EditorMode::Edit
    } else {
        mode
    };
    let spec = WindowSpec::new(
        EditorWindow::os_title(todo_id),
        gpui_kit::Bounds::new(Default::default(), logical_size(WIDTH, HEIGHT)),
    );
    WindowManager::open_child(
        cx,
        parent.as_ref(),
        WindowKind::Editor,
        None,
        spec,
        |window, cx| cx.new(|cx| EditorWindow::new(todo_id, mode, window, cx)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrant_color_follows_only_untouched_defaults() {
        // 新建默认色 = 象限 4 色：点"重要且紧急"颜色跟着变
        assert_eq!(
            resolve_quadrant_color(Some("#10B981"), 4, 1).as_deref(),
            Some("#EF4444")
        );
        // 大小写 / 空白不影响判断
        assert_eq!(
            resolve_quadrant_color(Some(" #10b981 "), 4, 2).as_deref(),
            Some("#F59E0B")
        );
        // 手动挑过的颜色保留
        assert_eq!(
            resolve_quadrant_color(Some("#6366F1"), 2, 1).as_deref(),
            Some("#6366F1")
        );
        assert_eq!(resolve_quadrant_color(None, 2, 1), None);
    }

    #[test]
    fn weekdays_parse_and_filter() {
        assert_eq!(parse_weekdays(Some("1,3,5")), vec![1, 3, 5]);
        assert_eq!(parse_weekdays(Some("0, 2,8,x,7")), vec![2, 7]);
        assert!(parse_weekdays(None).is_empty());
        assert!(parse_weekdays(Some("")).is_empty());
    }
}
