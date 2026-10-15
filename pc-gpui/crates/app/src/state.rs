//! 应用全局状态（替代旧前端的 Pinia `appStore` + `todoStore` 与跨窗口事件）。
//!
//! - 一个 `Entity<AppState>`，注册为 Global 句柄：[`AppState::global`]。所有窗口
//!   `cx.observe(&AppState::global(cx), |this, _, cx| cx.notify())` 跟着刷新。
//! - 写操作经 core 在后台执行，成功后更新本地副本并 `cx.notify()`；返回 `Task<Result<_, String>>`，
//!   调用方按旧版文案提示失败（`ui::message::error(window, cx, "更新待办失败", &e)`）。
//! - 需要整个主窗口提示的错误（加载待办失败）以 [`AppStateEvent::Message`] 发出，主窗口订阅后显示。
//! - 窗口模式（固定 / 嵌入 / 贴边隐藏）、窗口位置保存等平台动作由主窗口实现者（W1）实现，
//!   本文件只放状态字段与 TODO 标注的入口。

use std::path::PathBuf;

use futures::FutureExt as _;
use futures::future::Shared;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use mini_todo_core::db::{
    CreateSubTaskRequest, CreateTodoRequest, ScreenConfig, SubTask, Todo, UpdateSubTaskRequest,
    UpdateTodoRequest, WindowPosition, WindowSize,
};
use mini_todo_core::settings::{self, RuntimePrefs};
use mini_todo_core::sync::SyncReport;
use mini_todo_core::{Database, todo};
use serde::{Deserialize, Serialize};

use crate::db::{Db, spawn_db};
use crate::theme::{Palette, WindowBackground};
use crate::ui::message::MessageKind;

/// 应用版本（标题栏 `v2.5.0`）
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 版本检查用的 GitHub 仓库
pub const GITHUB_OWNER: &str = "dreamlonglll";
pub const GITHUB_REPO: &str = "mini-todo";
/// 待办默认颜色（新建表单 `DEFAULT_COLOR`）
pub const DEFAULT_TODO_COLOR: &str = "#10B981";

/// 列表 / 四象限
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    List,
    Quadrant,
}

impl ViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ViewMode::List => "list",
            ViewMode::Quadrant => "quadrant",
        }
    }

    /// settings `view_mode`：只接受 `list` / `quadrant`，其它值按 `list`
    pub fn parse(value: &str) -> ViewMode {
        match value {
            "quadrant" => ViewMode::Quadrant,
            _ => ViewMode::List,
        }
    }
}

/// 通知方式（settings `notification_type`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotificationType {
    #[default]
    System,
    App,
}

impl NotificationType {
    pub fn as_str(self) -> &'static str {
        match self {
            NotificationType::System => "system",
            NotificationType::App => "app",
        }
    }

    /// `app` 以外一律按 `system`
    pub fn parse(value: &str) -> NotificationType {
        if value == "app" {
            NotificationType::App
        } else {
            NotificationType::System
        }
    }
}

/// `todoStore.todoCount`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TodoCount {
    pub total: usize,
    pub pending: usize,
    pub completed: usize,
}

/// 主窗口需要弹出的提示（主窗口订阅后用 `ui::message` 显示）
#[derive(Debug, Clone, PartialEq)]
pub enum AppStateEvent {
    Message { kind: MessageKind, text: String },
}

/// 全局状态
pub struct AppState {
    // ---- todoStore ----
    /// 全部待办（含子任务），顺序同 `todo::list`（未完成在前、手动排序、新建在前）
    pub todos: Vec<Todo>,
    /// 列表 / 四象限（settings `view_mode`）
    pub view_mode: ViewMode,
    /// 上次成功加载时的本地变更计数（5s 轮询比对用）
    loaded_change_seq: Option<i64>,
    /// 本地写操作计数：拉取期间发生写操作则丢弃结果并补拉
    mutation_seq: u64,
    fetch_task: Option<Shared<Task<bool>>>,
    fetch_in_flight: bool,
    fetch_queued: bool,
    /// 连续加载失败只提示一次
    fetch_failing: bool,
    /// `change_seq` 读取失败只记一次日志
    change_seq_warned: bool,

    // ---- appStore ----
    /// 是否固定（`screen_configs.is_fixed`，按屏幕组合；含嵌入桌面）
    pub is_fixed: bool,
    /// 深色主题（settings `text_theme == "light"`，注意语义反转）
    pub is_dark_theme: bool,
    /// 主窗口位置 / 尺寸（物理像素；W1 维护）
    pub window_position: Option<WindowPosition>,
    pub window_size: Option<WindowSize>,
    /// 当前屏幕组合 ID（`platform::screen_config_id`；W1 维护）
    pub current_screen_config_id: String,
    pub screen_configs: Vec<ScreenConfig>,
    /// 展示日历（默认 false）
    pub show_calendar: bool,
    /// 贴边自动隐藏（默认 true）
    pub auto_hide_enabled: bool,
    /// 贴边唤起时置顶（默认 true）
    pub top_on_wake: bool,
    /// 固定模式时嵌入桌面（默认 false，仅 Windows）
    pub fixed_embed_desktop: bool,
    /// 深色主题下的窗口底色与透明度
    pub window_bg: WindowBackground,
    /// 展开全部子任务（旧版存在 localStorage，现存 `<数据目录>/ui-state.json`，不进库不同步）
    pub subtask_expand_all: bool,
    /// 待办字体（空 = 默认字体栈）/ 字号（12~20，默认 14；元信息行与子任务行 = 字号 − 2）
    pub todo_font_family: String,
    pub todo_font_size: i32,
    pub notification_type: NotificationType,
    /// 版本检查结果（W1）
    pub has_update: bool,
    pub latest_version: Option<String>,
    pub release_url: Option<String>,
    /// 贴边隐藏 / 唤起置顶的运行时偏好缓存（启动、导入、同步应用设置、保存窗口设置后刷新）
    pub runtime_prefs: RuntimePrefs,

    // ---- 运行时 ----
    /// 是否有同步在进行（标题栏同步图标旋转、按钮禁用）；手动 / 自动 / 设置页共用
    pub syncing: bool,
    syncing_count: usize,
    /// 自动同步处于连续失败段（同一段只提示一次；W1 维护）
    pub auto_sync_failing: bool,
    /// 最近一次同步报告（设置页显示"上次同步"时间用）
    pub last_sync_report: Option<SyncReport>,
    last_applied_sync_report: Option<SyncReport>,
    /// 设置与待办是否已完成首次加载
    pub initialized: bool,
}

impl EventEmitter<AppStateEvent> for AppState {}

struct AppStateGlobal(Entity<AppState>);
impl Global for AppStateGlobal {}

/// 一次性读出的设置（`reloadAppSettings` 顺序：viewMode → showCalendar → autoHide → fixedEmbedDesktop →
/// topOnWake → windowBackground → darkTheme → 字体；读失败回落默认值）
#[derive(Debug, Clone)]
struct LoadedSettings {
    view_mode: ViewMode,
    show_calendar: bool,
    auto_hide_enabled: bool,
    fixed_embed_desktop: bool,
    top_on_wake: bool,
    window_bg: WindowBackground,
    is_dark_theme: bool,
    todo_font_family: String,
    todo_font_size: i32,
    notification_type: NotificationType,
    screen_configs: Vec<ScreenConfig>,
    runtime_prefs: RuntimePrefs,
}

fn read_settings(db: &Database) -> LoadedSettings {
    let bg = settings::window_background(db).unwrap_or_else(|e| {
        log::warn!("[settings] 读取窗口底色失败: {e}");
        settings::WindowBackground {
            color: mini_todo_core::db::DEFAULT_WINDOW_BG_COLOR.into(),
            alpha: mini_todo_core::db::DEFAULT_WINDOW_BG_ALPHA,
        }
    });
    LoadedSettings {
        view_mode: ViewMode::parse(&settings::view_mode(db)),
        show_calendar: settings::show_calendar(db).unwrap_or(false),
        auto_hide_enabled: settings::auto_hide_enabled(db),
        fixed_embed_desktop: settings::fixed_embed_desktop(db),
        top_on_wake: settings::top_on_wake(db),
        window_bg: WindowBackground {
            color: bg.color,
            alpha: bg.alpha,
        },
        is_dark_theme: settings::text_theme(db).is_ok_and(|t| t == "light"),
        todo_font_family: settings::font_family(db).unwrap_or_default(),
        todo_font_size: settings::font_size(db).unwrap_or(14),
        notification_type: NotificationType::parse(
            &settings::notification_type(db).unwrap_or_default(),
        ),
        screen_configs: settings::list_screen_configs(db).unwrap_or_default(),
        runtime_prefs: settings::runtime_prefs(db),
    }
}

impl AppState {
    fn new() -> Self {
        Self {
            todos: Vec::new(),
            view_mode: ViewMode::List,
            loaded_change_seq: None,
            mutation_seq: 0,
            fetch_task: None,
            fetch_in_flight: false,
            fetch_queued: false,
            fetch_failing: false,
            change_seq_warned: false,
            is_fixed: false,
            is_dark_theme: false,
            window_position: None,
            window_size: None,
            current_screen_config_id: String::new(),
            screen_configs: Vec::new(),
            show_calendar: false,
            auto_hide_enabled: true,
            top_on_wake: true,
            fixed_embed_desktop: false,
            window_bg: WindowBackground::default(),
            subtask_expand_all: load_ui_state().subtask_expand_all,
            todo_font_family: String::new(),
            todo_font_size: 14,
            notification_type: NotificationType::System,
            has_update: false,
            latest_version: None,
            release_url: None,
            runtime_prefs: RuntimePrefs {
                top_on_wake: true,
                auto_hide_enabled: true,
            },
            syncing: false,
            syncing_count: 0,
            auto_sync_failing: false,
            last_sync_report: None,
            last_applied_sync_report: None,
            initialized: false,
        }
    }

    /// 创建全局状态（`main` 里在 `Db::init` 之后调用一次）
    pub fn init(cx: &mut App) -> Entity<AppState> {
        let state = cx.new(|_| AppState::new());
        cx.set_global(AppStateGlobal(state.clone()));
        state
    }

    /// 全局状态句柄
    pub fn global(cx: &App) -> Entity<AppState> {
        cx.global::<AppStateGlobal>().0.clone()
    }

    // ------------------------------------------------------------------
    // 派生值
    // ------------------------------------------------------------------

    /// 主窗口调色板（深色主题只在主窗口生效）
    pub fn main_palette(&self) -> Palette {
        Palette::for_main(self.is_dark_theme, &self.window_bg)
    }

    /// `isEmbeddedInDesktop = IS_WINDOWS && isFixed && fixedEmbedDesktop`
    pub fn is_embedded_in_desktop(&self) -> bool {
        cfg!(windows) && self.is_fixed && self.fixed_embed_desktop
    }

    /// 主窗口是否挂 `body.fixed-mode`（经典固定：无圆角、无描边）
    pub fn is_classic_fixed(&self) -> bool {
        self.is_fixed && !self.is_embedded_in_desktop()
    }

    /// 未完成待办，按 `sortOrder` 升序（同序保持后端顺序）
    pub fn pending_todos(&self) -> Vec<&Todo> {
        let mut list: Vec<&Todo> = self.todos.iter().filter(|t| !t.completed).collect();
        list.sort_by_key(|t| t.sort_order);
        list
    }

    /// 已完成待办，按 `sortOrder` 降序
    pub fn completed_todos(&self) -> Vec<&Todo> {
        let mut list: Vec<&Todo> = self.todos.iter().filter(|t| t.completed).collect();
        list.sort_by_key(|t| std::cmp::Reverse(t.sort_order));
        list
    }

    pub fn todo_count(&self) -> TodoCount {
        let completed = self.todos.iter().filter(|t| t.completed).count();
        TodoCount {
            total: self.todos.len(),
            pending: self.todos.len() - completed,
            completed,
        }
    }

    /// 四象限分组（仅未完成；下标 0..4 对应象限 1..4，非法象限归 1）
    pub fn todos_by_quadrant(&self) -> [Vec<&Todo>; 4] {
        let mut groups: [Vec<&Todo>; 4] = Default::default();
        for todo in self.pending_todos() {
            let index = match todo.quadrant {
                q @ 1..=4 => (q - 1) as usize,
                _ => 0,
            };
            groups[index].push(todo);
        }
        groups
    }

    pub fn todo(&self, id: i64) -> Option<&Todo> {
        self.todos.iter().find(|t| t.id == id)
    }

    // ------------------------------------------------------------------
    // 加载
    // ------------------------------------------------------------------

    /// 启动加载：设置（不含窗口定位，窗口定位由 W1 做）→ 待办。完成后 `initialized = true`。
    pub fn load_initial(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let reload = self.reload_app_settings(cx);
        cx.spawn(async move |this, cx| {
            reload.await;
            let fetch = this.update(cx, |this, cx| this.fetch_todos(cx));
            if let Ok(fetch) = fetch {
                fetch.await;
            }
            let _ = this.update(cx, |this, cx| {
                this.initialized = true;
                cx.notify();
            });
        })
    }

    /// 重新读取全部应用设置（旧 `reloadAppSettings`；导入、同步应用了远端设置、设置窗口关闭后调用）
    pub fn reload_app_settings(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let db = Db::get(cx);
        let task = cx
            .background_executor()
            .spawn(async move { read_settings(&db) });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let _ = this.update(cx, |this, cx| {
                this.view_mode = loaded.view_mode;
                this.show_calendar = loaded.show_calendar;
                this.auto_hide_enabled = loaded.auto_hide_enabled;
                this.fixed_embed_desktop = loaded.fixed_embed_desktop;
                this.top_on_wake = loaded.top_on_wake;
                this.window_bg = loaded.window_bg;
                this.is_dark_theme = loaded.is_dark_theme;
                this.todo_font_family = loaded.todo_font_family;
                this.todo_font_size = loaded.todo_font_size;
                this.notification_type = loaded.notification_type;
                this.screen_configs = loaded.screen_configs;
                this.runtime_prefs = loaded.runtime_prefs;
                cx.notify();
            });
        })
    }

    /// 拉取全部待办（旧 `todoStore.fetchTodos`）：
    /// - **单飞**：在途时新的调用只记"补拉"，返回同一个任务；结束后补拉一次，调用方拿到最终结果；
    /// - 每轮先读 `change_seq` 再读列表；期间发生本地写操作（`mutation_seq` 变化）则丢弃结果并补拉；
    /// - 连续失败只在第一次发出 `加载待办失败：…`（[`AppStateEvent::Message`]），成功后复位。
    ///
    /// 不需要结果时直接丢掉返回值即可（任务由状态持有，不会被取消）。
    pub fn fetch_todos(&mut self, cx: &mut Context<Self>) -> Shared<Task<bool>> {
        if self.fetch_in_flight
            && let Some(task) = &self.fetch_task
        {
            self.fetch_queued = true;
            return task.clone();
        }
        self.fetch_in_flight = true;
        let task = cx
            .spawn(async move |this, cx| {
                let mut ok = false;
                loop {
                    let Ok((start_mutation, db)) = this.update(cx, |this, cx| {
                        this.fetch_queued = false;
                        (this.mutation_seq, Db::get(cx))
                    }) else {
                        return false;
                    };
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let seq = todo::change_seq(&db);
                            todo::list(&db).map(|todos| (seq, todos))
                        })
                        .await;
                    let Ok(again) = this.update(cx, |this, cx| {
                        match result {
                            Ok(_) if this.mutation_seq != start_mutation => {
                                // 拉取期间有本地写操作：结果可能是旧的，丢弃并补拉
                                this.fetch_queued = true;
                            }
                            Ok((seq, todos)) => {
                                this.todos = todos;
                                match seq {
                                    Ok(seq) => this.loaded_change_seq = Some(seq),
                                    Err(e) => {
                                        this.loaded_change_seq = None;
                                        if !this.change_seq_warned {
                                            this.change_seq_warned = true;
                                            log::warn!("[todos] 读取变更计数失败: {e}");
                                        }
                                    }
                                }
                                this.fetch_failing = false;
                                ok = true;
                                cx.notify();
                            }
                            Err(e) => {
                                ok = false;
                                log::error!("[todos] 加载待办失败: {e}");
                                if !this.fetch_failing {
                                    this.fetch_failing = true;
                                    cx.emit(AppStateEvent::Message {
                                        kind: MessageKind::Error,
                                        text: crate::ui::message::error_text("加载待办失败", &e),
                                    });
                                }
                            }
                        }
                        let again = this.fetch_queued;
                        if !again {
                            this.fetch_in_flight = false;
                        }
                        again
                    }) else {
                        return false;
                    };
                    if !again {
                        return ok;
                    }
                }
            })
            .shared();
        self.fetch_task = Some(task.clone());
        task
    }

    /// [`Self::fetch_todos`] 的"不关心结果"版本
    pub fn reload_todos(&mut self, cx: &mut Context<Self>) {
        drop(self.fetch_todos(cx));
    }

    /// 5s 轮询（旧 `refreshIfChanged`）：拉取在途则跳过；`change_seq` 读不到或没变则跳过；否则 `fetch_todos`。
    /// 模态子窗口打开期间由调用方（主窗口）跳过。
    pub fn refresh_if_changed(&mut self, cx: &mut Context<Self>) {
        if self.fetch_in_flight {
            return;
        }
        let loaded = self.loaded_change_seq;
        let task = spawn_db(cx, todo::change_seq);
        cx.spawn(async move |this, cx| {
            let seq = match task.await {
                Ok(seq) => seq,
                Err(e) => {
                    let _ = this.update(cx, |this, _| {
                        if !this.change_seq_warned {
                            this.change_seq_warned = true;
                            log::warn!("[todos] 读取变更计数失败: {e}");
                        }
                    });
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| {
                if Some(seq) != loaded && !this.fetch_in_flight {
                    this.reload_todos(cx);
                }
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // 待办写操作（成功后更新本地副本；失败原样返回 core 的错误文本）
    // ------------------------------------------------------------------

    /// 新建待办（`create_todo`）；成功后插入本地列表
    pub fn create_todo(
        &mut self,
        request: CreateTodoRequest,
        cx: &mut Context<Self>,
    ) -> Task<Result<Todo, String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| todo::create(db, &request));
        cx.spawn(async move |this, cx| {
            let created = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.upsert_local(created.clone());
                cx.notify();
            });
            Ok(created)
        })
    }

    /// 更新待办（`update_todo`，失败文案前缀 `更新待办失败`）；用返回值替换本地项
    pub fn update_todo(
        &mut self,
        id: i64,
        request: UpdateTodoRequest,
        cx: &mut Context<Self>,
    ) -> Task<Result<Todo, String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| todo::update(db, id, &request));
        cx.spawn(async move |this, cx| {
            let updated = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.upsert_local(updated.clone());
                cx.notify();
            });
            Ok(updated)
        })
    }

    /// 切换完成态（`updateTodo(id, {completed: !completed})`）
    pub fn toggle_complete(
        &mut self,
        id: i64,
        cx: &mut Context<Self>,
    ) -> Task<Result<Todo, String>> {
        let completed = self.todo(id).is_some_and(|t| t.completed);
        self.update_todo(
            id,
            UpdateTodoRequest {
                completed: Some(!completed),
                ..empty_update()
            },
            cx,
        )
    }

    /// 改象限（四象限拖拽；`color` 为 Some 时同时改颜色）
    pub fn update_todo_quadrant(
        &mut self,
        id: i64,
        quadrant: i32,
        color: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<Todo, String>> {
        self.update_todo(
            id,
            UpdateTodoRequest {
                quadrant: Some(quadrant),
                color,
                ..empty_update()
            },
            cx,
        )
    }

    /// 删除待办（连带子任务，写墓碑；失败文案前缀 `删除待办失败`）
    pub fn delete_todo(&mut self, id: i64, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| todo::delete(db, id));
        cx.spawn(async move |this, cx| {
            task.await?;
            let _ = this.update(cx, |this, cx| {
                this.todos.retain(|t| t.id != id);
                cx.notify();
            });
            Ok(())
        })
    }

    /// 保存排序（下标即 `sortOrder`；失败文案前缀 `保存排序失败`）。本地立即生效
    pub fn reorder_todos(
        &mut self,
        ids: Vec<i64>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.mutation_seq += 1;
        for (index, id) in ids.iter().enumerate() {
            if let Some(todo) = self.todos.iter_mut().find(|t| t.id == *id) {
                todo.sort_order = index as i32;
            }
        }
        cx.notify();
        let task = spawn_db(cx, move |db| todo::reorder(db, &ids));
        cx.spawn(async move |_, _| task.await)
    }

    // ------------------------------------------------------------------
    // 子任务写操作（成功后同步到本地的父待办）
    // ------------------------------------------------------------------

    pub fn create_subtask(
        &mut self,
        request: CreateSubTaskRequest,
        cx: &mut Context<Self>,
    ) -> Task<Result<SubTask, String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| todo::create_subtask(db, &request));
        cx.spawn(async move |this, cx| {
            let created = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.upsert_local_subtask(created.clone());
                cx.notify();
            });
            Ok(created)
        })
    }

    pub fn update_subtask(
        &mut self,
        id: i64,
        request: UpdateSubTaskRequest,
        cx: &mut Context<Self>,
    ) -> Task<Result<SubTask, String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| todo::update_subtask(db, id, &request));
        cx.spawn(async move |this, cx| {
            let updated = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.upsert_local_subtask(updated.clone());
                cx.notify();
            });
            Ok(updated)
        })
    }

    pub fn delete_subtask(&mut self, id: i64, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| todo::delete_subtask(db, id));
        cx.spawn(async move |this, cx| {
            task.await?;
            let _ = this.update(cx, |this, cx| {
                for todo in &mut this.todos {
                    todo.subtasks.retain(|s| s.id != id);
                }
                cx.notify();
            });
            Ok(())
        })
    }

    /// 子任务排序（下标即 `sortOrder`）；本地立即生效
    pub fn reorder_subtasks(
        &mut self,
        ids: Vec<i64>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.mutation_seq += 1;
        for todo in &mut self.todos {
            let mut touched = false;
            for subtask in &mut todo.subtasks {
                if let Some(index) = ids.iter().position(|id| *id == subtask.id) {
                    subtask.sort_order = index as i32;
                    touched = true;
                }
            }
            if touched {
                todo.subtasks.sort_by_key(|s| (s.sort_order, s.id));
            }
        }
        cx.notify();
        let task = spawn_db(cx, move |db| todo::reorder_subtasks(db, &ids));
        cx.spawn(async move |_, _| task.await)
    }

    /// 只刷新一条待办（编辑窗口的子任务操作后、子任务窗口关闭后；旧 `loadTodo(false)`）
    pub fn reload_todo(&mut self, id: i64, cx: &mut Context<Self>) -> Task<Result<Todo, String>> {
        let task = spawn_db(cx, move |db| todo::get(db, id));
        cx.spawn(async move |this, cx| {
            let loaded = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.upsert_local(loaded.clone());
                cx.notify();
            });
            Ok(loaded)
        })
    }

    fn upsert_local(&mut self, todo: Todo) {
        match self.todos.iter_mut().find(|t| t.id == todo.id) {
            Some(existing) => *existing = todo,
            None => self.todos.push(todo),
        }
    }

    fn upsert_local_subtask(&mut self, subtask: SubTask) {
        let Some(parent) = self.todos.iter_mut().find(|t| t.id == subtask.parent_id) else {
            return;
        };
        match parent.subtasks.iter_mut().find(|s| s.id == subtask.id) {
            Some(existing) => *existing = subtask,
            None => parent.subtasks.push(subtask),
        }
        parent.subtasks.sort_by_key(|s| (s.sort_order, s.id));
    }

    // ------------------------------------------------------------------
    // 设置（先改状态再写库，失败回滚并返回带前缀的提示文本，调用方 `ui::message::error_text_only` 显示）
    // ------------------------------------------------------------------

    /// 视图模式（失败只记日志，与旧版一致）
    pub fn set_view_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.view_mode = mode;
        cx.notify();
        let task = spawn_db(cx, move |db| settings::set_view_mode(db, mode.as_str()));
        cx.spawn(async move |_, _| {
            if let Err(e) = task.await {
                log::warn!("[settings] 保存视图模式失败: {e}");
            }
        })
        .detach();
    }

    /// 深色主题（`set_text_theme`：开 = `light`）；失败回滚，错误文本 `设置深色主题失败：…`
    pub fn set_dark_theme(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let previous = self.is_dark_theme;
        self.is_dark_theme = enabled;
        cx.notify();
        let theme = if enabled { "light" } else { "dark" };
        let task = spawn_db(cx, move |db| settings::set_text_theme(db, theme));
        self.rollback_on_error(
            task,
            "设置深色主题失败",
            move |this| this.is_dark_theme = previous,
            cx,
        )
    }

    /// 展示日历；错误文本 `设置日历显示失败：…`
    pub fn set_show_calendar(
        &mut self,
        show: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let previous = self.show_calendar;
        self.show_calendar = show;
        cx.notify();
        let task = spawn_db(cx, move |db| settings::set_show_calendar(db, show));
        self.rollback_on_error(
            task,
            "设置日历显示失败",
            move |this| this.show_calendar = previous,
            cx,
        )
    }

    /// 贴边自动隐藏；错误文本 `设置贴边自动隐藏失败：…`。
    /// 状态机（`AutoHideState::set_enabled`）的收尾由 W1 在观察到变化后处理。
    pub fn set_auto_hide_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let previous = self.auto_hide_enabled;
        self.auto_hide_enabled = enabled;
        self.runtime_prefs.auto_hide_enabled = enabled;
        cx.notify();
        let task = spawn_db(cx, move |db| settings::set_auto_hide_enabled(db, enabled));
        self.rollback_on_error(
            task,
            "设置贴边自动隐藏失败",
            move |this| {
                this.auto_hide_enabled = previous;
                this.runtime_prefs.auto_hide_enabled = previous;
            },
            cx,
        )
    }

    /// 唤起时置顶；错误文本 `设置唤起置顶失败：…`（关闭时立即撤置顶由 W1 处理）
    pub fn set_top_on_wake(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let previous = self.top_on_wake;
        self.top_on_wake = enabled;
        self.runtime_prefs.top_on_wake = enabled;
        cx.notify();
        let task = spawn_db(cx, move |db| settings::set_top_on_wake(db, enabled));
        self.rollback_on_error(
            task,
            "设置唤起置顶失败",
            move |this| {
                this.top_on_wake = previous;
                this.runtime_prefs.top_on_wake = previous;
            },
            cx,
        )
    }

    /// 固定模式时嵌入桌面（只写键；已固定时切换窗口形态由 W1 观察到变化后处理）；
    /// 错误文本 `设置嵌入桌面失败：…`
    pub fn set_fixed_embed_desktop(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let previous = self.fixed_embed_desktop;
        self.fixed_embed_desktop = enabled;
        cx.notify();
        let task = spawn_db(cx, move |db| settings::set_fixed_embed_desktop(db, enabled));
        self.rollback_on_error(
            task,
            "设置嵌入桌面失败",
            move |this| this.fixed_embed_desktop = previous,
            cx,
        )
    }

    /// 窗口底色 / 透明度（透明度夹到 0~1）；错误文本 `设置界面底色失败：…`
    pub fn set_window_background(
        &mut self,
        color: String,
        alpha: f64,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let previous = self.window_bg.clone();
        let alpha = alpha.clamp(0.0, 1.0);
        self.window_bg = WindowBackground {
            color: color.clone(),
            alpha,
        };
        cx.notify();
        let task = spawn_db(cx, move |db| {
            settings::set_window_background(db, &color, alpha)
        });
        self.rollback_on_error(
            task,
            "设置界面底色失败",
            move |this| this.window_bg = previous,
            cx,
        )
    }

    /// 待办字体与字号（字号夹到 12~20）
    pub fn set_todo_font(
        &mut self,
        family: String,
        size: i32,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let size = size.clamp(12, 20);
        self.todo_font_family = family.clone();
        self.todo_font_size = size;
        cx.notify();
        spawn_db(cx, move |db| {
            settings::set_font_family(db, &family)?;
            settings::set_font_size(db, size)
        })
    }

    /// 通知方式
    pub fn set_notification_type(
        &mut self,
        value: NotificationType,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.notification_type = value;
        cx.notify();
        spawn_db(cx, move |db| {
            settings::set_notification_type(db, value.as_str())
        })
    }

    /// 展开 / 收起全部子任务（存 `ui-state.json`）
    pub fn toggle_subtask_expand_all(&mut self, cx: &mut Context<Self>) {
        self.subtask_expand_all = !self.subtask_expand_all;
        cx.notify();
        let state = UiState {
            subtask_expand_all: self.subtask_expand_all,
        };
        cx.background_executor()
            .spawn(async move { save_ui_state(&state) })
            .detach();
    }

    /// 重新读屏幕配置列表（设置窗口「屏幕配置」）
    pub fn reload_screen_configs(&mut self, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        let task = spawn_db(cx, settings::list_screen_configs);
        cx.spawn(async move |this, cx| {
            let configs = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.screen_configs = configs;
                cx.notify();
            });
            Ok(())
        })
    }

    fn rollback_on_error(
        &mut self,
        task: Task<Result<(), String>>,
        prefix: &'static str,
        rollback: impl FnOnce(&mut AppState) + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Err(e) = &result {
                log::error!("[settings] {prefix}: {e}");
                let _ = this.update(cx, |this, cx| {
                    rollback(this);
                    cx.notify();
                });
            }
            result.map_err(|e| crate::ui::message::error_text(prefix, &e))
        })
    }

    // ------------------------------------------------------------------
    // 同步（入口在 services::sync::run_sync）
    // ------------------------------------------------------------------

    pub(crate) fn begin_sync(&mut self, cx: &mut Context<Self>) {
        self.syncing_count += 1;
        self.syncing = true;
        cx.notify();
    }

    pub(crate) fn end_sync(&mut self, cx: &mut Context<Self>) {
        self.syncing_count = self.syncing_count.saturating_sub(1);
        self.syncing = self.syncing_count > 0;
        cx.notify();
    }

    /// 处理同步报告（旧 `applySyncReport`）：同一份报告只处理一次；本地数据有变化且主窗口没有
    /// 模态子窗口 → 重新拉待办；应用了远端设置 → 重新读设置（含运行时偏好）。返回是否处理了。
    pub fn apply_sync_report(&mut self, report: &SyncReport, cx: &mut Context<Self>) -> bool {
        self.last_sync_report = Some(report.clone());
        if self.last_applied_sync_report.as_ref() == Some(report) {
            return false;
        }
        self.last_applied_sync_report = Some(report.clone());
        if crate::services::sync::has_local_data_changes(report)
            && !crate::windows::WindowManager::main_has_modal_child(cx)
        {
            self.reload_todos(cx);
        }
        if report.settings_applied {
            self.reload_app_settings(cx).detach();
        }
        cx.notify();
        true
    }

    // ------------------------------------------------------------------
    // 平台相关动作（W1 实现）
    // ------------------------------------------------------------------

    /// 切换固定模式（旧 `toggleFixedMode`）：翻转 `is_fixed` → 按"嵌入桌面"偏好进入经典固定 / 嵌入桌面，
    /// 或退回普通模式 → 保存窗口状态；失败提示 `切换固定模式失败：…`（W1：`services::window_mode`）
    pub fn toggle_fixed_mode(&mut self, cx: &mut Context<Self>) {
        self.is_fixed = !self.is_fixed;
        let fixed = self.is_fixed;
        cx.notify();
        cx.defer(move |cx| crate::services::window_mode::toggle_fixed(cx, fixed));
    }

    /// 保存主窗口状态（旧 `saveWindowState`：读窗口物理几何（贴边隐藏中用锚点）→
    /// `settings::save_screen_config` → `settings::save_window_settings` → 刷新运行时偏好）。
    /// 设置窗口里不要调用（几何取的是主窗口，但旧版约定设置项各写各的键）
    pub fn save_window_state(&mut self, cx: &mut Context<Self>) {
        cx.defer(crate::services::window_mode::save_window_state);
    }
}

fn empty_update() -> UpdateTodoRequest {
    UpdateTodoRequest {
        title: None,
        description: None,
        color: None,
        quadrant: None,
        notify_at: None,
        notify_before: None,
        completed: None,
        sort_order: None,
        clear_notify_at: false,
        start_time: None,
        end_time: None,
        clear_start_time: false,
        clear_end_time: false,
        repeat_enabled: None,
        repeat_type: None,
        repeat_interval: None,
        repeat_weekdays: None,
        repeat_month_day: None,
        clear_repeat: false,
    }
}

/// 所有字段为空的 `UpdateTodoRequest`（只改给出的字段）
pub fn update_request() -> UpdateTodoRequest {
    empty_update()
}

// ----------------------------------------------------------------------
// ui-state.json（替代 localStorage，只存纯界面偏好，不进库不同步）
// ----------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct UiState {
    subtask_expand_all: bool,
}

fn ui_state_path() -> PathBuf {
    mini_todo_core::db::paths::app_data_dir().join("ui-state.json")
}

fn load_ui_state() -> UiState {
    std::fs::read_to_string(ui_state_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_ui_state(state: &UiState) {
    let path = ui_state_path();
    let result = serde_json::to_string_pretty(state)
        .map_err(|e| e.to_string())
        .and_then(|text| std::fs::write(&path, text).map_err(|e| e.to_string()));
    if let Err(e) = result {
        log::warn!("[ui-state] 写入 {} 失败: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(id: i64, completed: bool, sort_order: i32, quadrant: i32) -> Todo {
        Todo {
            id,
            title: format!("t{id}"),
            description: None,
            color: "#EF4444".into(),
            quadrant,
            notify_at: None,
            notify_before: 0,
            notified: false,
            completed,
            sort_order,
            start_time: None,
            end_time: None,
            created_at: String::new(),
            updated_at: String::new(),
            repeat_enabled: false,
            repeat_type: None,
            repeat_interval: 1,
            repeat_weekdays: None,
            repeat_month_day: None,
            subtasks: vec![],
        }
    }

    #[test]
    fn derived_lists_follow_todo_store() {
        let mut state = AppState::new();
        state.todos = vec![
            todo(1, false, 2, 1),
            todo(2, true, 0, 2),
            todo(3, false, 0, 9),
            todo(4, true, 5, 3),
            todo(5, false, 1, 4),
        ];
        let pending: Vec<i64> = state.pending_todos().iter().map(|t| t.id).collect();
        assert_eq!(pending, vec![3, 5, 1]);
        let completed: Vec<i64> = state.completed_todos().iter().map(|t| t.id).collect();
        assert_eq!(completed, vec![4, 2]);
        assert_eq!(
            state.todo_count(),
            TodoCount {
                total: 5,
                pending: 3,
                completed: 2
            }
        );
        let groups = state.todos_by_quadrant();
        assert_eq!(
            groups[0].iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![3, 1]
        );
        assert_eq!(groups[3].iter().map(|t| t.id).collect::<Vec<_>>(), vec![5]);
        assert!(groups[1].is_empty() && groups[2].is_empty());
    }

    #[test]
    fn view_mode_and_notification_type_parse() {
        assert_eq!(ViewMode::parse("quadrant"), ViewMode::Quadrant);
        assert_eq!(ViewMode::parse("grid"), ViewMode::List);
        assert_eq!(NotificationType::parse("app"), NotificationType::App);
        assert_eq!(
            NotificationType::parse("whatever"),
            NotificationType::System
        );
    }

    #[test]
    fn embedded_only_on_windows_when_fixed() {
        let mut state = AppState::new();
        state.fixed_embed_desktop = true;
        assert!(!state.is_embedded_in_desktop());
        state.is_fixed = true;
        assert_eq!(state.is_embedded_in_desktop(), cfg!(windows));
        assert_eq!(state.is_classic_fixed(), !cfg!(windows));
    }
}
