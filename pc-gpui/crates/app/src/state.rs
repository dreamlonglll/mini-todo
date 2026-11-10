//! 应用全局状态（替代旧前端的 Pinia `appStore` + `todoStore` 与跨窗口事件）。
//!
//! - 一个 `Entity<AppState>`，注册为 Global 句柄：[`AppState::global`]。所有窗口
//!   `cx.observe(&AppState::global(cx), |this, _, cx| cx.notify())` 跟着刷新。
//! - 写操作经 core 在后台执行，成功后更新本地副本并 `cx.notify()`；返回 `Task<Result<_, String>>`，
//!   调用方按旧版文案提示失败（`ui::message::error(window, cx, "更新待办失败", &e)`）。
//! - 需要整个应用提示的错误（加载待办失败）以 [`AppStateEvent::Message`] 发出，`services::notice` 订阅后显示在看得见的清单 /
//!   日历窗口里。
//! - 窗口模式（锁定 / 嵌入 / 贴边隐藏）、窗口位置保存等平台动作在 `services::window_mode`：锁定按窗口
//!   （`window_mode::is_fixed(cx, key)` / `toggle_fixed(cx, key)`），这里只放全局的偏好（嵌入桌面、贴边隐藏、唤起置顶）。
//! - **待办清单**（v31，[`AppState::lists`]）随待办一起加载（5s 轮询、同步完成、子窗口关闭后都走 [`AppState::fetch_todos`]）；
//!   本机新建 / 改名 / 删除走 [`AppState::create_list`] 等。清单列表每被替换一次 [`AppState::lists_revision`] 加一，
//!   `services::block_windows` 观察它开 / 关清单窗口。待办按**有效清单**（`lists::effective_list_id`：归属未知或清单不存在时
//!   按默认清单）归到各清单窗口。
//! - **窗口属性按窗口**（清单 / 日历窗口的隐藏、日 / 夜、列表 / 四象限、展开全部、日历筛选）：core `window_prefs` 的内存副本，
//!   读 [`AppState::window_prefs`]，写 [`AppState::set_window_dark`] 等（先改内存、后台写库，失败回滚）。同步键 `view_mode` /
//!   `text_theme` / `show_calendar` / `is_fixed` 只作迁移 v31 的初始值，不再驱动界面、也不再写入。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use futures::FutureExt as _;
use futures::future::Shared;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use mini_todo_core::db::{
    CreateSubTaskRequest, CreateTodoRequest, ScreenConfig, SubTask, Todo, TodoList,
    UpdateSubTaskRequest, UpdateTodoRequest,
};
use mini_todo_core::settings::{self, RuntimePrefs};
use mini_todo_core::sync::SyncReport;
use mini_todo_core::window_prefs::{self, UpdateWindowPrefs, WindowPrefs};
use mini_todo_core::{Database, lists, todo};

pub use mini_todo_core::window_prefs::ViewMode;

use crate::db::{Db, spawn_db};
use crate::theme::{Palette, WindowBackground};
use crate::ui::message::MessageKind;
use crate::windows::block::{BlockKey, DEFAULT_LIST_ID};

/// 应用版本（标题栏 `v2.5.0`）
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 版本检查用的 GitHub 仓库
pub const GITHUB_OWNER: &str = "dreamlonglll";
pub const GITHUB_REPO: &str = "mini-todo";
/// 待办默认颜色（新建表单 `DEFAULT_COLOR`）
pub const DEFAULT_TODO_COLOR: &str = "#10B981";
/// 托盘「新增待办清单」的默认名称；重名时加序号（"清单 2"、"清单 3"……）
pub const NEW_LIST_NAME: &str = "清单";

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

/// 需要在窗口里弹出的提示（`services::notice` 订阅后用 `ui::message` 显示在看得见的清单 / 日历窗口里）
#[derive(Debug, Clone, PartialEq)]
pub enum AppStateEvent {
    Message { kind: MessageKind, text: String },
}

/// 全局状态
pub struct AppState {
    // ---- todoStore ----
    /// 全部待办（含子任务），顺序同 `todo::list`（未完成在前、手动排序、新建在前）
    pub todos: Vec<Todo>,
    /// 上次成功加载时的本地变更计数（5s 轮询比对用）
    loaded_change_seq: Option<i64>,
    /// 本地写操作计数（待办 / 子任务 / 清单）：拉取期间发生写操作则丢弃结果并补拉
    mutation_seq: u64,
    /// 本地写窗口属性的计数：拉取期间写过窗口属性时，这次读到的窗口属性不覆盖内存副本
    prefs_mutation_seq: u64,
    /// 写日历筛选的计数：只有最后一次写入的结果（core 规范化后的值 / 失败回滚）落到内存副本
    calendar_filter_seq: u64,
    fetch_task: Option<Shared<Task<bool>>>,
    fetch_in_flight: bool,
    fetch_queued: bool,
    /// 连续加载失败只提示一次
    fetch_failing: bool,
    /// `change_seq` 读取失败只记一次日志
    change_seq_warned: bool,

    // ---- 待办清单与窗口属性（v31） ----
    /// 全部清单（`lists::list`：按 `(sortOrder, id)`，含默认清单）
    pub lists: Vec<TodoList>,
    /// 清单列表 / 窗口属性记录每被外部替换一次（加载、同步、本机新建 / 改名 / 删除）加一：`services::block_windows`
    /// 观察它，给同步来的新清单开窗、关掉已删除清单的窗口
    pub lists_revision: u64,
    /// 清单 / 日历窗口的设备本地属性（core `window_prefs` 的内存副本；没有记录的窗口按 `WindowPrefs::new` 的默认值）
    window_prefs: BTreeMap<BlockKey, WindowPrefs>,

    // ---- appStore ----
    // 是否固定（锁定）按窗口，在 `services::window_mode`（按屏幕组合持久化，见 `window_mode::store`）；
    // 日 / 夜、列表 / 四象限、展开全部、是否显示日历按窗口，在 `window_prefs`
    /// 当前屏幕组合 ID（`platform::screen_config_id`；`services::window_mode` 维护）
    pub current_screen_config_id: String,
    pub screen_configs: Vec<ScreenConfig>,
    /// 贴边自动隐藏（默认 true）
    pub auto_hide_enabled: bool,
    /// 贴边唤起时置顶（默认 true）
    pub top_on_wake: bool,
    /// 固定模式时嵌入桌面（默认 false，仅 Windows）
    pub fixed_embed_desktop: bool,
    /// 深色主题下的窗口底色与透明度（全局，作用于每个深色的清单 / 日历窗口）
    pub window_bg: WindowBackground,
    /// 待办字体（空 = 默认字体栈）/ 字号（12~20，默认 14；元信息行与子任务行 = 字号 − 2）
    pub todo_font_family: String,
    pub todo_font_size: i32,
    pub notification_type: NotificationType,
    /// 版本检查结果（`services::update_check`）
    pub has_update: bool,
    pub latest_version: Option<String>,
    pub release_url: Option<String>,
    /// 贴边隐藏 / 唤起置顶的运行时偏好缓存（启动、导入、同步应用设置后刷新）
    pub runtime_prefs: RuntimePrefs,

    // ---- 运行时 ----
    /// 是否有同步在进行（标题栏同步图标旋转、按钮禁用）；手动 / 自动 / 设置页共用
    pub syncing: bool,
    syncing_count: usize,
    /// 自动同步处于连续失败段（同一段只提示一次；`services::auto_sync` 维护）
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

/// 一次性读出的设置（`reloadAppSettings` 顺序：autoHide → fixedEmbedDesktop → topOnWake → windowBackground → 字体；
/// 读失败回落默认值）。`view_mode` / `show_calendar` / `text_theme` 改为按窗口（`window_prefs`），不再从全局键读
#[derive(Debug, Clone)]
struct LoadedSettings {
    auto_hide_enabled: bool,
    fixed_embed_desktop: bool,
    top_on_wake: bool,
    window_bg: WindowBackground,
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
        auto_hide_enabled: settings::auto_hide_enabled(db),
        fixed_embed_desktop: settings::fixed_embed_desktop(db),
        top_on_wake: settings::top_on_wake(db),
        window_bg: WindowBackground {
            color: bg.color,
            alpha: bg.alpha,
        },
        todo_font_family: settings::font_family(db).unwrap_or_default(),
        todo_font_size: settings::font_size(db).unwrap_or(14),
        notification_type: NotificationType::parse(
            &settings::notification_type(db).unwrap_or_default(),
        ),
        screen_configs: settings::list_screen_configs(db).unwrap_or_default(),
        runtime_prefs: settings::runtime_prefs(db),
    }
}

/// 一次拉取的结果（待办 + 清单 + 窗口属性；后两项读失败时为 `None`，保留内存里原来的）
struct Loaded {
    seq: Result<i64, String>,
    todos: Vec<Todo>,
    lists: Option<Vec<TodoList>>,
    prefs: Option<Vec<WindowPrefs>>,
}

fn load_all(db: &Database) -> Result<Loaded, String> {
    let seq = todo::change_seq(db);
    let todos = todo::list(db)?;
    let (lists, prefs) = load_lists(db);
    Ok(Loaded {
        seq,
        todos,
        lists,
        prefs,
    })
}

/// 读清单与窗口属性（读失败只记日志，返回 `None` = 不覆盖内存副本）
fn load_lists(db: &Database) -> (Option<Vec<TodoList>>, Option<Vec<WindowPrefs>>) {
    let lists = lists::list(db)
        .map_err(|e| log::warn!("[lists] 读取待办清单失败: {e}"))
        .ok();
    let prefs = window_prefs::list_prefs(db)
        .map_err(|e| log::warn!("[window-prefs] 读取窗口属性失败: {e}"))
        .ok();
    (lists, prefs)
}

impl AppState {
    fn new() -> Self {
        Self {
            todos: Vec::new(),
            loaded_change_seq: None,
            mutation_seq: 0,
            prefs_mutation_seq: 0,
            calendar_filter_seq: 0,
            fetch_task: None,
            fetch_in_flight: false,
            fetch_queued: false,
            fetch_failing: false,
            change_seq_warned: false,
            lists: Vec::new(),
            lists_revision: 0,
            window_prefs: BTreeMap::new(),
            current_screen_config_id: String::new(),
            screen_configs: Vec::new(),
            auto_hide_enabled: true,
            top_on_wake: true,
            fixed_embed_desktop: false,
            window_bg: WindowBackground::default(),
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
    // 派生值：待办（按有效清单）
    // ------------------------------------------------------------------

    /// 待办的有效清单（`lists::effective_list_id`：归属未知或清单不存在时按默认清单）
    pub fn effective_list_id(&self, todo: &Todo) -> i64 {
        lists::effective_list_id(todo.list_id, &self.lists)
    }

    /// 清单 `list_id` 里的未完成待办，按 `sortOrder` 升序（同序保持后端顺序）
    pub fn pending_todos_in(&self, list_id: i64) -> Vec<&Todo> {
        let mut list: Vec<&Todo> = todos_in_list(&self.todos, &self.lists, list_id)
            .filter(|t| !t.completed)
            .collect();
        list.sort_by_key(|t| t.sort_order);
        list
    }

    /// 清单 `list_id` 的待办计数
    pub fn todo_count_in(&self, list_id: i64) -> TodoCount {
        let (mut total, mut completed) = (0, 0);
        for todo in todos_in_list(&self.todos, &self.lists, list_id) {
            total += 1;
            if todo.completed {
                completed += 1;
            }
        }
        TodoCount {
            total,
            pending: total - completed,
            completed,
        }
    }

    /// 清单 `list_id` 的四象限分组（仅未完成；下标 0..4 对应象限 1..4，非法象限归 1）
    pub fn todos_by_quadrant_in(&self, list_id: i64) -> [Vec<&Todo>; 4] {
        let mut groups: [Vec<&Todo>; 4] = Default::default();
        for todo in self.pending_todos_in(list_id) {
            let index = match todo.quadrant {
                q @ 1..=4 => (q - 1) as usize,
                _ => 0,
            };
            groups[index].push(todo);
        }
        groups
    }

    /// 日历窗口显示的待办：所有清单（含已完成），按日历的清单筛选（[`Self::calendar_filter`]；"全部"时不过滤）
    pub fn calendar_todos(&self) -> Vec<&Todo> {
        let filter = self.calendar_filter();
        self.todos
            .iter()
            .filter(|t| in_calendar_filter(t, &self.lists, filter.as_deref()))
            .collect()
    }

    /// 日历的清单筛选（PRD R7；`None` = 全部，含以后新建的清单）：只算现有的清单，空了按"全部"
    /// （[`effective_calendar_filter`]；本地 / 同步删除清单后 core 也会清理存储的筛选，内存副本可能晚一步）
    pub fn calendar_filter(&self) -> Option<Vec<i64>> {
        let prefs = self.window_prefs.get(&BlockKey::Calendar)?;
        effective_calendar_filter(prefs.calendar_filter.as_deref(), &self.lists)
    }

    pub fn todo(&self, id: i64) -> Option<&Todo> {
        self.todos.iter().find(|t| t.id == id)
    }

    // ------------------------------------------------------------------
    // 派生值：清单与窗口属性
    // ------------------------------------------------------------------

    pub fn list(&self, id: i64) -> Option<&TodoList> {
        self.lists.iter().find(|l| l.id == id)
    }

    /// 清单名称（找不到时：默认清单按迁移种子的名字，其它为空）
    pub fn list_name(&self, id: i64) -> String {
        match self.list(id) {
            Some(list) => list.name.clone(),
            None if id == DEFAULT_LIST_ID => lists::DEFAULT_LIST_NAME.to_string(),
            None => String::new(),
        }
    }

    /// 窗口 `key` 的属性（没有记录时是 `WindowPrefs::new` 的默认值：显示、浅色、列表、不展开、全部清单）
    pub fn window_prefs(&self, key: BlockKey) -> WindowPrefs {
        self.window_prefs
            .get(&key)
            .cloned()
            .unwrap_or_else(|| WindowPrefs::new(key.window_key()))
    }

    /// 本机有没有窗口 `key` 的属性记录（同步来的新清单没有：自动开窗）
    pub fn has_window_record(&self, key: BlockKey) -> bool {
        self.window_prefs.contains_key(&key)
    }

    /// 窗口 `key` 是否深色主题
    pub fn is_dark(&self, key: BlockKey) -> bool {
        self.window_prefs.get(&key).is_some_and(|p| p.dark)
    }

    /// 窗口 `key` 的调色板（深色时用全局的底色与透明度）
    pub fn palette_for(&self, key: BlockKey) -> Palette {
        Palette::for_main(self.is_dark(key), &self.window_bg)
    }

    /// 清单窗口 `key` 的视图模式
    pub fn view_mode(&self, key: BlockKey) -> ViewMode {
        self.window_prefs
            .get(&key)
            .map_or(ViewMode::List, |p| p.view_mode)
    }

    /// 清单窗口 `key` 是否展开全部子任务
    pub fn expand_all(&self, key: BlockKey) -> bool {
        self.window_prefs.get(&key).is_some_and(|p| p.expand_all)
    }

    /// 是否显示日历窗口（日历窗口的属性 `!hidden`；设置 → 常规「显示日历」与托盘勾选同一个状态）
    pub fn calendar_visible(&self) -> bool {
        !self.window_prefs(BlockKey::Calendar).hidden
    }

    // ------------------------------------------------------------------
    // 加载
    // ------------------------------------------------------------------

    /// 启动时同步读到的清单与窗口属性（建窗前就要知道深色 / 隐藏，第一帧按它画；`services::block_windows` 调用）
    pub fn set_startup_snapshot(&mut self, lists: Vec<TodoList>, prefs: Vec<WindowPrefs>) {
        self.lists = lists;
        self.window_prefs = prefs_map(prefs);
    }

    /// 启动加载：设置（不含窗口定位，窗口定位在 `services::window_mode`）→ 待办。完成后 `initialized = true`。
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
                this.auto_hide_enabled = loaded.auto_hide_enabled;
                this.fixed_embed_desktop = loaded.fixed_embed_desktop;
                this.top_on_wake = loaded.top_on_wake;
                this.window_bg = loaded.window_bg;
                this.todo_font_family = loaded.todo_font_family;
                this.todo_font_size = loaded.todo_font_size;
                this.notification_type = loaded.notification_type;
                this.screen_configs = loaded.screen_configs;
                this.runtime_prefs = loaded.runtime_prefs;
                cx.notify();
            });
        })
    }

    /// 拉取全部待办（旧 `todoStore.fetchTodos`），连同待办清单与窗口属性：
    /// - **单飞**：在途时新的调用只记"补拉"，返回同一个任务；结束后补拉一次，调用方拿到最终结果；
    /// - 每轮先读 `change_seq` 再读列表；期间发生本地写操作（`mutation_seq` 变化）则丢弃结果并补拉；期间写过窗口属性
    ///   （`prefs_mutation_seq` 变化）时这次读到的窗口属性不覆盖内存副本；
    /// - 清单列表或窗口属性记录有变化时 `lists_revision` 加一；
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
                    let Ok((start_mutation, start_prefs, db)) = this.update(cx, |this, cx| {
                        this.fetch_queued = false;
                        (this.mutation_seq, this.prefs_mutation_seq, Db::get(cx))
                    }) else {
                        return false;
                    };
                    let result = cx
                        .background_executor()
                        .spawn(async move { load_all(&db) })
                        .await;
                    let Ok(again) = this.update(cx, |this, cx| {
                        match result {
                            Ok(_) if this.mutation_seq != start_mutation => {
                                // 拉取期间有本地写操作：结果可能是旧的，丢弃并补拉
                                this.fetch_queued = true;
                            }
                            Ok(loaded) => {
                                this.todos = loaded.todos;
                                match loaded.seq {
                                    Ok(seq) => this.loaded_change_seq = Some(seq),
                                    Err(e) => {
                                        this.loaded_change_seq = None;
                                        if !this.change_seq_warned {
                                            this.change_seq_warned = true;
                                            log::warn!("[todos] 读取变更计数失败: {e}");
                                        }
                                    }
                                }
                                let prefs = loaded
                                    .prefs
                                    .filter(|_| this.prefs_mutation_seq == start_prefs);
                                this.apply_loaded_lists(loaded.lists, prefs);
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

    /// 拉取到的清单 / 窗口属性（`None` = 这次没读到或不该覆盖）替换内存副本；有变化时 `lists_revision` 加一
    fn apply_loaded_lists(
        &mut self,
        lists: Option<Vec<TodoList>>,
        prefs: Option<Vec<WindowPrefs>>,
    ) {
        let mut changed = false;
        if let Some(lists) = lists
            && lists != self.lists
        {
            self.lists = lists;
            changed = true;
        }
        if let Some(prefs) = prefs {
            let prefs = prefs_map(prefs);
            if prefs != self.window_prefs {
                self.window_prefs = prefs;
                changed = true;
            }
        }
        if changed {
            self.lists_revision += 1;
        }
    }

    /// [`Self::fetch_todos`] 的"不关心结果"版本
    pub fn reload_todos(&mut self, cx: &mut Context<Self>) {
        drop(self.fetch_todos(cx));
    }

    /// 只重读清单与窗口属性、不拉待办（有窗口开着模态子窗口、待办的重拉要推迟时）：同步来的清单照样开窗、被删清单照样关窗
    /// （`lists_revision` → `services::block_windows::reconcile`），编辑窗口的「清单」下拉也跟着更新。
    /// 完整拉取在途时只记补拉（它的结果可能早于这次同步，补拉的那一轮会带上新清单）；读的期间有本地写操作则丢弃，交给之后的完整重拉
    fn reload_lists(&mut self, cx: &mut Context<Self>) {
        if self.fetch_in_flight {
            self.fetch_queued = true;
            return;
        }
        let (start_mutation, start_prefs) = (self.mutation_seq, self.prefs_mutation_seq);
        let task = spawn_db(cx, |db| Ok(load_lists(db)));
        cx.spawn(async move |this, cx| {
            let Ok((lists, prefs)) = task.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.mutation_seq != start_mutation {
                    return;
                }
                let prefs = prefs.filter(|_| this.prefs_mutation_seq == start_prefs);
                let revision = this.lists_revision;
                this.apply_loaded_lists(lists, prefs);
                if this.lists_revision != revision {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 5s 轮询（旧 `refreshIfChanged`）：拉取在途则跳过；`change_seq` 读不到或没变则跳过；否则 `fetch_todos`。
    /// 有模态子窗口时由调用方（`services::block_windows` 的轮询）跳过。清单的增删改名同样计入 `change_seq`
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

    /// 把待办移到另一个清单（待办行右键「移动到」；core `todo::move_to_list`：只改清单归属，归属的版本与 `updatedAt`
    /// 都刷新，随同步传播）。本地立即生效：本清单窗口里这条随即消失、目标清单窗口里出现；成功后用库里的结果替换本地项，
    /// 失败把本地的归属改回去（没被别处改过时）并原样返回 core 的错误文本（失败文案前缀 `移动待办失败`）
    pub fn move_todo_to_list(
        &mut self,
        id: i64,
        list_id: i64,
        cx: &mut Context<Self>,
    ) -> Task<Result<Todo, String>> {
        self.mutation_seq += 1;
        let previous = self.set_local_list_id(id, Some(list_id));
        cx.notify();
        let task = spawn_db(cx, move |db| todo::move_to_list(db, id, list_id));
        cx.spawn(async move |this, cx| match task.await {
            Ok(moved) => {
                let _ = this.update(cx, |this, cx| {
                    this.upsert_local(moved.clone());
                    cx.notify();
                });
                Ok(moved)
            }
            Err(e) => {
                let _ = this.update(cx, |this, cx| {
                    if let Some(previous) = previous
                        && this.todo(id).is_some_and(|t| t.list_id == Some(list_id))
                    {
                        this.set_local_list_id(id, previous);
                        cx.notify();
                    }
                });
                Err(e)
            }
        })
    }

    /// 本地副本里把待办 `id` 的清单归属改成 `list_id`，返回原来的归属（没有这条待办时 `None`）
    fn set_local_list_id(&mut self, id: i64, list_id: Option<i64>) -> Option<Option<i64>> {
        let todo = self.todos.iter_mut().find(|t| t.id == id)?;
        Some(std::mem::replace(&mut todo.list_id, list_id))
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

    /// 保存排序（下标即 `sortOrder`；失败文案前缀 `保存排序失败`）。本地立即生效。
    /// 清单窗口只传本清单的待办：`sortOrder` 只在同一清单里比较，与其它清单的值重复不影响
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
    // 待办清单写操作（core `lists`；成功后更新本地副本、`lists_revision` 加一）
    // ------------------------------------------------------------------

    /// 新建清单（排在最后）。`name` 为 `None` 时自动取名"清单 / 清单 2 / …"（按库里最新的清单避开重名，见
    /// [`next_list_name`]）。同一个后台任务里按默认值建好它的窗口属性记录（显示、浅色、列表视图）：随后由调用方开窗，
    /// 观察清单列表的 `services::block_windows` 不会把它当成"同步来的新清单"另开一个
    pub fn create_list(
        &mut self,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<TodoList, String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| {
            let name = match name {
                Some(name) => name,
                None => {
                    let existing = lists::list(db)?;
                    next_list_name(existing.iter().map(|l| l.name.as_str()))
                }
            };
            let created = lists::create(db, &name)?;
            let prefs = window_prefs::update_prefs(
                db,
                BlockKey::List(created.id).window_key(),
                &UpdateWindowPrefs::default(),
            )?;
            Ok((created, prefs))
        });
        cx.spawn(async move |this, cx| {
            let (created, prefs) = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.upsert_local_list(created.clone());
                if let Some(prefs) = prefs {
                    this.window_prefs
                        .insert(BlockKey::from_window_key(prefs.key), prefs);
                }
                this.lists_revision += 1;
                cx.notify();
            });
            Ok(created)
        })
    }

    /// 清单改名（默认清单也可以；名称规则见 `lists::normalize_name`）
    pub fn rename_list(
        &mut self,
        id: i64,
        name: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<TodoList, String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| lists::rename(db, id, &name));
        cx.spawn(async move |this, cx| {
            let renamed = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.upsert_local_list(renamed.clone());
                this.lists_revision += 1;
                cx.notify();
            });
            Ok(renamed)
        })
    }

    /// 删除清单（默认清单不行）：其下待办移到默认清单、写清单墓碑、删窗口属性（core `lists::delete`）。
    /// 返回移到默认清单的待办数；成功后重新拉待办（待办的归属变了）
    pub fn delete_list(&mut self, id: i64, cx: &mut Context<Self>) -> Task<Result<usize, String>> {
        self.mutation_seq += 1;
        let task = spawn_db(cx, move |db| lists::delete(db, id));
        cx.spawn(async move |this, cx| {
            let moved = task.await?;
            let _ = this.update(cx, |this, cx| {
                this.lists.retain(|l| l.id != id);
                this.window_prefs.remove(&BlockKey::List(id));
                // 同 core `lists::delete`：从日历筛选里移出，空了回到"全部"
                let lists = &this.lists;
                if let Some(prefs) = this.window_prefs.get_mut(&BlockKey::Calendar) {
                    prefs.calendar_filter =
                        effective_calendar_filter(prefs.calendar_filter.as_deref(), lists);
                }
                this.lists_revision += 1;
                this.reload_todos(cx);
                cx.notify();
            });
            Ok(moved)
        })
    }

    fn upsert_local_list(&mut self, list: TodoList) {
        match self.lists.iter_mut().find(|l| l.id == list.id) {
            Some(existing) => *existing = list,
            None => self.lists.push(list),
        }
        self.lists.sort_by_key(|l| (l.sort_order, l.id));
    }

    // ------------------------------------------------------------------
    // 窗口属性（先改内存再后台写库，失败回滚；core `window_prefs::update_prefs`）
    // ------------------------------------------------------------------

    /// 窗口 `key` 的日 / 夜；错误文本 `设置深色主题失败：…`
    pub fn set_window_dark(
        &mut self,
        key: BlockKey,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.update_window_prefs(
            key,
            UpdateWindowPrefs {
                dark: Some(dark),
                ..Default::default()
            },
            "设置深色主题失败",
            cx,
        )
    }

    /// 清单窗口 `key` 的列表 / 四象限（失败只记日志，与旧版一致）
    pub fn set_window_view_mode(&mut self, key: BlockKey, mode: ViewMode, cx: &mut Context<Self>) {
        self.update_window_prefs(
            key,
            UpdateWindowPrefs {
                view_mode: Some(mode),
                ..Default::default()
            },
            "保存视图模式失败",
            cx,
        )
        .detach();
    }

    /// 展开 / 收起清单窗口 `key` 的全部子任务（失败只记日志）
    pub fn toggle_window_expand_all(&mut self, key: BlockKey, cx: &mut Context<Self>) {
        let expand_all = !self.expand_all(key);
        self.update_window_prefs(
            key,
            UpdateWindowPrefs {
                expand_all: Some(expand_all),
                ..Default::default()
            },
            "保存展开全部子任务失败",
            cx,
        )
        .detach();
    }

    /// 窗口 `key` 是否隐藏（关闭 = 隐藏；日历窗口的隐藏 = 「显示日历」关闭）。错误文本 `保存窗口状态失败：…`
    pub fn set_window_hidden(
        &mut self,
        key: BlockKey,
        hidden: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.update_window_prefs(
            key,
            UpdateWindowPrefs {
                hidden: Some(hidden),
                ..Default::default()
            },
            "保存窗口状态失败",
            cx,
        )
    }

    /// 日历的清单筛选（`None` = 全部）；core 去掉不存在的清单、去重排序，空了回到全部。错误文本 `保存日历筛选失败：…`。
    /// 连续点筛选时只有最后一次的结果落到内存副本（前一次写库的结果晚到不覆盖后一次的选择）
    pub fn set_calendar_filter(
        &mut self,
        filter: Option<Vec<i64>>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.prefs_mutation_seq += 1;
        self.calendar_filter_seq += 1;
        let seq = self.calendar_filter_seq;
        let key = BlockKey::Calendar;
        let previous = self.window_prefs.get(&key).cloned();
        let mut next = self.window_prefs(key);
        next.calendar_filter = filter.clone();
        self.window_prefs.insert(key, next);
        cx.notify();
        let task = spawn_db(cx, move |db| {
            window_prefs::set_calendar_filter(db, filter.as_deref())
        });
        cx.spawn(async move |this, cx| match task.await {
            Ok(saved) => {
                let _ = this.update(cx, |this, cx| {
                    if this.calendar_filter_seq == seq
                        && let Some(prefs) = this.window_prefs.get_mut(&key)
                    {
                        prefs.calendar_filter = saved;
                        cx.notify();
                    }
                });
                Ok(())
            }
            Err(e) => {
                log::error!("[window-prefs] 保存日历筛选失败: {e}");
                let _ = this.update(cx, |this, cx| {
                    if this.calendar_filter_seq == seq {
                        restore_prefs(&mut this.window_prefs, key, previous);
                        cx.notify();
                    }
                });
                Err(crate::ui::message::error_text("保存日历筛选失败", &e))
            }
        })
    }

    fn update_window_prefs(
        &mut self,
        key: BlockKey,
        update: UpdateWindowPrefs,
        prefix: &'static str,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.prefs_mutation_seq += 1;
        let previous = self.window_prefs.get(&key).cloned();
        let mut next = self.window_prefs(key);
        apply_prefs_update(&mut next, &update);
        self.window_prefs.insert(key, next);
        cx.notify();
        let window_key = key.window_key();
        let task = spawn_db(cx, move |db| {
            window_prefs::update_prefs(db, window_key, &update).map(|_| ())
        });
        self.rollback_on_error(
            task,
            prefix,
            move |this| restore_prefs(&mut this.window_prefs, key, previous),
            cx,
        )
    }

    // ------------------------------------------------------------------
    // 设置（先改状态再写库，失败回滚并返回带前缀的提示文本，调用方 `ui::message::error_text_only` 显示）
    // ------------------------------------------------------------------

    /// 贴边自动隐藏；错误文本 `设置贴边自动隐藏失败：…`。
    /// 状态机（`AutoHideState::set_enabled`）的收尾由 `services::window_mode` 观察到变化后处理。
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

    /// 唤起时置顶；错误文本 `设置唤起置顶失败：…`（关闭时立即撤置顶由 `services::window_mode` 处理）
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

    /// 固定模式时嵌入桌面（只写键；已固定时切换窗口形态由 `services::window_mode` 观察到变化后处理）；
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

    /// 处理同步报告（旧 `applySyncReport`）：同一份报告只处理一次；本地数据（待办 / 子任务 / 清单）有变化且没有窗口开着
    /// 模态子窗口 → 重新拉待办（连同清单与窗口属性：同步来的新清单开窗、被删清单关窗由 `services::block_windows` 处理）；
    /// 开着模态子窗口时待办的重拉推迟到子窗口关闭，但清单有变化时先只重读清单（[`Self::reload_lists`]），开 / 关窗不等；
    /// 应用了远端设置 → 重新读设置（含运行时偏好）。返回是否处理了。
    pub fn apply_sync_report(&mut self, report: &SyncReport, cx: &mut Context<Self>) -> bool {
        self.last_sync_report = Some(report.clone());
        if self.last_applied_sync_report.as_ref() == Some(report) {
            return false;
        }
        self.last_applied_sync_report = Some(report.clone());
        if crate::services::sync::has_local_data_changes(report) {
            if !crate::windows::WindowManager::any_block_has_modal_child(cx) {
                self.reload_todos(cx);
            } else if report.lists_changed() {
                self.reload_lists(cx);
            }
        }
        if report.settings_applied {
            self.reload_app_settings(cx).detach();
        }
        cx.notify();
        true
    }
}

/// `todos` 里有效清单是 `list_id` 的待办（`lists::effective_list_id`：归属未知 / 清单不存在 → 默认清单）
pub fn todos_in_list<'a>(
    todos: &'a [Todo],
    lists: &'a [TodoList],
    list_id: i64,
) -> impl Iterator<Item = &'a Todo> + 'a {
    todos
        .iter()
        .filter(move |t| lists::effective_list_id(t.list_id, lists) == list_id)
}

/// 日历的清单筛选（契约 / PRD R7）：`None` = 全部；否则待办的有效清单在筛选里（兜底到默认清单的待办按默认清单参与）
pub fn in_calendar_filter(todo: &Todo, lists: &[TodoList], filter: Option<&[i64]>) -> bool {
    filter.is_none_or(|ids| ids.contains(&lists::effective_list_id(todo.list_id, lists)))
}

/// 实际生效的日历筛选（PRD R7）：去掉已不存在（被删除）的清单，空了回到"全部"（`None`）；"全部"保持"全部"
/// （以后新建的清单也在里面），具体的一组清单不会自动加上新清单
pub fn effective_calendar_filter(filter: Option<&[i64]>, lists: &[TodoList]) -> Option<Vec<i64>> {
    let kept: Vec<i64> = filter?
        .iter()
        .copied()
        .filter(|id| lists.iter().any(|l| l.id == *id))
        .collect();
    (!kept.is_empty()).then_some(kept)
}

/// 新清单的默认名称："清单" 没被占用就用它，否则从 2 开始找第一个空闲序号（去首尾空白后不区分大小写比较，同
/// `lists::names_conflict`）
pub fn next_list_name<'a>(existing: impl IntoIterator<Item = &'a str>) -> String {
    let existing: Vec<&str> = existing.into_iter().collect();
    let taken = |name: &str| existing.iter().any(|e| lists::names_conflict(e, name));
    if !taken(NEW_LIST_NAME) {
        return NEW_LIST_NAME.to_string();
    }
    (2..)
        .map(|n| format!("{NEW_LIST_NAME} {n}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| NEW_LIST_NAME.to_string())
}

/// 把 [`UpdateWindowPrefs`] 里给出的字段写进 `prefs`（与 core `update_prefs` 的 COALESCE 语义相同）
pub fn apply_prefs_update(prefs: &mut WindowPrefs, update: &UpdateWindowPrefs) {
    if let Some(hidden) = update.hidden {
        prefs.hidden = hidden;
    }
    if let Some(dark) = update.dark {
        prefs.dark = dark;
    }
    if let Some(view_mode) = update.view_mode {
        prefs.view_mode = view_mode;
    }
    if let Some(expand_all) = update.expand_all {
        prefs.expand_all = expand_all;
    }
}

fn prefs_map(prefs: Vec<WindowPrefs>) -> BTreeMap<BlockKey, WindowPrefs> {
    prefs
        .into_iter()
        .map(|p| (BlockKey::from_window_key(p.key), p))
        .collect()
}

fn restore_prefs(
    map: &mut BTreeMap<BlockKey, WindowPrefs>,
    key: BlockKey,
    previous: Option<WindowPrefs>,
) {
    match previous {
        Some(prefs) => {
            map.insert(key, prefs);
        }
        None => {
            map.remove(&key);
        }
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
        list_id: None,
    }
}

/// 所有字段为空的 `UpdateTodoRequest`（只改给出的字段）
pub fn update_request() -> UpdateTodoRequest {
    empty_update()
}

// ----------------------------------------------------------------------
// ui-state.json：3.0.x 把全局的「展开全部子任务」存在这里（`subtaskExpandAll`，替代旧版 localStorage）；现在按窗口存在
// core 的窗口属性里，只剩一次性迁移
// ----------------------------------------------------------------------

/// 3.0.x 写在 ui-state.json 里的键
const LEGACY_EXPAND_ALL_KEY: &str = "subtaskExpandAll";

/// ui-state.json 里待迁移的旧值
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyUiState {
    /// 旧的全局「展开全部子任务」（不是布尔值时按 false，同旧版 `#[serde(default)]` 读不出来的结果）
    pub expand_all: bool,
    /// 去掉这个键后文件剩下的内容；`None` = 什么都不剩，删掉文件
    pub rest: Option<String>,
}

/// 从 ui-state.json 的内容里取出旧的「展开全部子任务」。文件不是 JSON 对象或没有这个键时 `None`（不用迁移）
pub fn take_legacy_expand_all(text: &str) -> Option<LegacyUiState> {
    let serde_json::Value::Object(mut object) = serde_json::from_str(text).ok()? else {
        return None;
    };
    let value = object.remove(LEGACY_EXPAND_ALL_KEY)?;
    let rest = (!object.is_empty()).then(|| {
        serde_json::to_string_pretty(&serde_json::Value::Object(object)).unwrap_or_default()
    });
    Some(LegacyUiState {
        expand_all: value.as_bool().unwrap_or(false),
        rest,
    })
}

fn ui_state_path() -> PathBuf {
    mini_todo_core::db::paths::app_data_dir().join("ui-state.json")
}

/// 启动时（建窗前，阻塞）把 ui-state.json 里旧的全局「展开全部子任务」迁移到默认清单窗口（`list:1` 的 `expand_all`）。
///
/// **迁移标记 = 这个键不在了**：写进窗口属性成功后才把 `subtaskExpandAll` 从文件里删掉（文件只剩它时删文件），写库失败
/// 留着下次启动再迁。回退到 3.0.x 又改了展开全部（旧版会重新写这个键）、再升级回来时按旧版最后的值再迁一次
pub fn migrate_legacy_ui_state(db: &Database) {
    migrate_legacy_ui_state_at(db, &ui_state_path());
}

fn migrate_legacy_ui_state_at(db: &Database, path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Some(legacy) = take_legacy_expand_all(&text) else {
        return;
    };
    let update = UpdateWindowPrefs {
        expand_all: Some(legacy.expand_all),
        ..Default::default()
    };
    if let Err(e) = window_prefs::update_prefs(db, BlockKey::DEFAULT_LIST.window_key(), &update) {
        log::warn!("[ui-state] 迁移「展开全部子任务」失败，下次启动再试: {e}");
        return;
    }
    let result = match &legacy.rest {
        Some(rest) => std::fs::write(path, rest),
        None => std::fs::remove_file(path),
    };
    match result {
        Ok(()) => log::info!(
            "[ui-state] 已把「展开全部子任务」（{}）迁移到默认清单窗口",
            legacy.expand_all
        ),
        Err(e) => log::warn!("[ui-state] 迁移后改写 {} 失败: {e}", path.display()),
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
            list_id: Some(1),
            list_updated_at: None,
            subtasks: vec![],
        }
    }

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
    fn derived_lists_follow_todo_store_per_list() {
        let mut state = AppState::new();
        state.lists = vec![list(1, "待办清单"), list(7, "工作")];
        state.todos = vec![
            todo(1, false, 2, 1),
            todo(2, true, 0, 2),
            todo(3, false, 0, 9),
            todo(4, true, 5, 3),
            todo(5, false, 1, 4),
            Todo {
                list_id: Some(7),
                ..todo(6, false, 0, 2)
            },
            Todo {
                list_id: Some(7),
                ..todo(7, true, 3, 2)
            },
        ];
        let pending: Vec<i64> = state.pending_todos_in(1).iter().map(|t| t.id).collect();
        assert_eq!(pending, vec![3, 5, 1]);
        assert_eq!(
            state.todo_count_in(1),
            TodoCount {
                total: 5,
                pending: 3,
                completed: 2
            }
        );
        let groups = state.todos_by_quadrant_in(1);
        assert_eq!(
            groups[0].iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![3, 1]
        );
        assert_eq!(groups[3].iter().map(|t| t.id).collect::<Vec<_>>(), vec![5]);
        assert!(groups[1].is_empty() && groups[2].is_empty());

        let work: Vec<i64> = state.pending_todos_in(7).iter().map(|t| t.id).collect();
        assert_eq!(work, vec![6]);
        assert_eq!(state.todo_count_in(7).completed, 1);
        assert_eq!(state.todos_by_quadrant_in(7)[1].len(), 1);
    }

    #[test]
    fn moving_a_todo_takes_it_out_of_its_list_at_once_and_can_be_undone() {
        let mut state = AppState::new();
        state.lists = vec![list(1, "待办清单"), list(7, "工作")];
        state.todos = vec![
            Todo {
                list_id: None,
                ..todo(1, false, 0, 1)
            },
            todo(2, false, 1, 1),
        ];
        let pending = |state: &AppState, list_id| -> Vec<i64> {
            state
                .pending_todos_in(list_id)
                .iter()
                .map(|t| t.id)
                .collect()
        };
        // 归属未知的待办移到「工作」：默认清单窗口里随即消失，「工作」窗口里出现
        assert_eq!(state.set_local_list_id(1, Some(7)), Some(None));
        assert_eq!(pending(&state, 1), vec![2]);
        assert_eq!(pending(&state, 7), vec![1]);
        // 失败回滚：改回原来的（未知）
        assert_eq!(state.set_local_list_id(1, None), Some(Some(7)));
        assert_eq!(pending(&state, 1), vec![1, 2]);
        assert_eq!(state.set_local_list_id(99, Some(7)), None, "没有这条待办");
    }

    #[test]
    fn todos_go_to_their_effective_list() {
        let lists = vec![list(1, "待办清单"), list(7, "工作")];
        let todos = vec![
            Todo {
                list_id: Some(7),
                ..todo(1, false, 0, 1)
            },
            // 归属未知（旧版本 / 迁移前的存量）→ 默认清单
            Todo {
                list_id: None,
                ..todo(2, false, 0, 1)
            },
            // 清单已删除 / 不存在 → 默认清单（记录不改写）
            Todo {
                list_id: Some(99),
                ..todo(3, false, 0, 1)
            },
            todo(4, false, 0, 1),
        ];
        let ids = |list_id| -> Vec<i64> {
            todos_in_list(&todos, &lists, list_id)
                .map(|t| t.id)
                .collect()
        };
        assert_eq!(ids(7), vec![1]);
        assert_eq!(ids(1), vec![2, 3, 4]);
        assert!(ids(99).is_empty(), "不存在的清单没有待办");
    }

    #[test]
    fn calendar_filter_uses_the_effective_list() {
        let lists = vec![list(1, "待办清单"), list(7, "工作"), list(8, "生活")];
        let work = Todo {
            list_id: Some(7),
            ..todo(1, false, 0, 1)
        };
        let unknown = Todo {
            list_id: None,
            ..todo(2, false, 0, 1)
        };
        let deleted = Todo {
            list_id: Some(99),
            ..todo(3, false, 0, 1)
        };
        for t in [&work, &unknown, &deleted] {
            assert!(in_calendar_filter(t, &lists, None), "全部");
        }
        assert!(in_calendar_filter(&work, &lists, Some(&[7, 8])));
        assert!(!in_calendar_filter(&unknown, &lists, Some(&[7, 8])));
        // 兜底到默认清单的待办按默认清单参与筛选
        assert!(in_calendar_filter(&unknown, &lists, Some(&[1])));
        assert!(in_calendar_filter(&deleted, &lists, Some(&[1])));
        assert!(!in_calendar_filter(&work, &lists, Some(&[1])));
    }

    #[test]
    fn calendar_filter_drops_deleted_lists_and_falls_back_to_all() {
        let lists = vec![list(1, "待办清单"), list(7, "工作"), list(8, "生活")];
        assert_eq!(effective_calendar_filter(None, &lists), None, "全部");
        assert_eq!(
            effective_calendar_filter(Some(&[7, 99]), &lists),
            Some(vec![7]),
            "被删除的清单移出"
        );
        assert_eq!(
            effective_calendar_filter(Some(&[99]), &lists),
            None,
            "空了回到全部"
        );

        let mut state = AppState::new();
        state.lists = lists;
        state.todos = vec![
            Todo {
                list_id: Some(7),
                ..todo(1, false, 0, 1)
            },
            Todo {
                list_id: Some(8),
                ..todo(2, false, 0, 1)
            },
        ];
        let shown = |state: &AppState| -> Vec<i64> {
            state.calendar_todos().iter().map(|t| t.id).collect()
        };
        assert_eq!(state.calendar_filter(), None, "没有日历的记录：全部");
        assert_eq!(shown(&state), vec![1, 2]);
        state.window_prefs.insert(
            BlockKey::Calendar,
            WindowPrefs {
                calendar_filter: Some(vec![8]),
                ..WindowPrefs::new(BlockKey::Calendar.window_key())
            },
        );
        assert_eq!(shown(&state), vec![2]);
        // 「生活」被删（内存里的筛选还没清理）：筛选空了，按全部显示
        state.lists.retain(|l| l.id != 8);
        assert_eq!(state.calendar_filter(), None);
        assert_eq!(shown(&state), vec![1, 2]);
    }

    #[test]
    fn new_list_names_skip_taken_ones_ignoring_case_and_spaces() {
        assert_eq!(next_list_name(["待办清单"]), "清单");
        assert_eq!(next_list_name(["待办清单", " 清单 "]), "清单 2");
        assert_eq!(next_list_name(["清单", "清单 2", "清单 4"]), "清单 3");
        assert_eq!(next_list_name(Vec::<&str>::new()), "清单");
        assert_eq!(next_list_name(["清单", "清单 2", "清单 3"]), "清单 4");
    }

    #[test]
    fn window_prefs_default_until_a_record_exists() {
        let mut state = AppState::new();
        let key = BlockKey::List(7);
        assert!(!state.has_window_record(key));
        assert_eq!(state.window_prefs(key), WindowPrefs::new(key.window_key()));
        assert!(!state.is_dark(key));
        assert_eq!(state.view_mode(key), ViewMode::List);
        assert!(!state.expand_all(key));
        assert!(state.calendar_visible(), "没有记录的日历按显示");

        state.set_startup_snapshot(
            vec![list(1, "待办清单")],
            vec![
                WindowPrefs {
                    dark: true,
                    view_mode: ViewMode::Quadrant,
                    expand_all: true,
                    ..WindowPrefs::new(BlockKey::DEFAULT_LIST.window_key())
                },
                WindowPrefs {
                    hidden: true,
                    ..WindowPrefs::new(BlockKey::Calendar.window_key())
                },
            ],
        );
        assert!(state.has_window_record(BlockKey::DEFAULT_LIST));
        assert!(state.is_dark(BlockKey::DEFAULT_LIST));
        assert!(state.palette_for(BlockKey::DEFAULT_LIST).is_dark);
        assert!(!state.palette_for(BlockKey::Calendar).is_dark);
        assert_eq!(state.view_mode(BlockKey::DEFAULT_LIST), ViewMode::Quadrant);
        assert!(state.expand_all(BlockKey::DEFAULT_LIST));
        assert!(!state.calendar_visible());
        assert_eq!(state.list_name(1), "待办清单");
        assert_eq!(state.list_name(42), "");
    }

    #[test]
    fn loaded_lists_and_prefs_bump_the_revision_only_when_they_change() {
        let mut state = AppState::new();
        state.set_startup_snapshot(vec![list(1, "待办清单")], vec![]);
        state.apply_loaded_lists(Some(vec![list(1, "待办清单")]), Some(vec![]));
        assert_eq!(state.lists_revision, 0, "没变化");
        state.apply_loaded_lists(Some(vec![list(1, "待办清单"), list(7, "工作")]), None);
        assert_eq!(state.lists_revision, 1);
        assert_eq!(state.lists.len(), 2);
        state.apply_loaded_lists(
            None,
            Some(vec![WindowPrefs::new(BlockKey::List(7).window_key())]),
        );
        assert_eq!(state.lists_revision, 2, "窗口属性记录出现");
        assert!(state.has_window_record(BlockKey::List(7)));
        state.apply_loaded_lists(None, None);
        assert_eq!(state.lists_revision, 2, "没读到时保留原来的");
        assert_eq!(state.lists.len(), 2);
    }

    #[test]
    fn prefs_updates_change_only_the_given_fields() {
        let mut prefs = WindowPrefs::new(BlockKey::DEFAULT_LIST.window_key());
        apply_prefs_update(
            &mut prefs,
            &UpdateWindowPrefs {
                dark: Some(true),
                ..Default::default()
            },
        );
        apply_prefs_update(
            &mut prefs,
            &UpdateWindowPrefs {
                hidden: Some(true),
                view_mode: Some(ViewMode::Quadrant),
                expand_all: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(
            prefs,
            WindowPrefs {
                hidden: true,
                dark: true,
                view_mode: ViewMode::Quadrant,
                expand_all: true,
                ..WindowPrefs::new(BlockKey::DEFAULT_LIST.window_key())
            }
        );
    }

    #[test]
    fn legacy_ui_state_is_taken_out_of_the_file() {
        assert_eq!(
            take_legacy_expand_all("{\n  \"subtaskExpandAll\": true\n}"),
            Some(LegacyUiState {
                expand_all: true,
                rest: None
            })
        );
        // 文件里还有别的键（以后的版本 / 别的东西写的）：只去掉这一个
        let taken = take_legacy_expand_all(r#"{"subtaskExpandAll": false, "other": 1}"#).unwrap();
        assert!(!taken.expand_all);
        let rest: serde_json::Value = serde_json::from_str(&taken.rest.unwrap()).unwrap();
        assert_eq!(rest, serde_json::json!({ "other": 1 }));
        // 不是布尔值：同旧版读不出来，按 false
        assert_eq!(
            take_legacy_expand_all(r#"{"subtaskExpandAll": "yes"}"#).map(|l| l.expand_all),
            Some(false)
        );
        // 已迁移（没有这个键）/ 不是 JSON 对象：不用迁移
        assert_eq!(take_legacy_expand_all("{}"), None);
        assert_eq!(take_legacy_expand_all(r#"{"other": 1}"#), None);
        assert_eq!(take_legacy_expand_all("garbage"), None);
        assert_eq!(take_legacy_expand_all("[true]"), None);
    }

    #[test]
    fn legacy_ui_state_migrates_into_the_default_list_window_once() {
        let db = Database::new_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "mini-todo-ui-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ui-state.json");
        let expand_all = |db: &Database| {
            window_prefs::get_prefs(db, BlockKey::DEFAULT_LIST.window_key())
                .unwrap()
                .unwrap()
                .expand_all
        };

        std::fs::write(&path, "{\n  \"subtaskExpandAll\": true\n}").unwrap();
        migrate_legacy_ui_state_at(&db, &path);
        assert!(expand_all(&db));
        assert!(!path.exists(), "只剩这个键：删掉文件（= 已迁移）");

        // 之后用户在默认清单窗口里收起了；再启动不会被旧文件覆盖（文件已经没了）
        window_prefs::update_prefs(
            &db,
            BlockKey::DEFAULT_LIST.window_key(),
            &UpdateWindowPrefs {
                expand_all: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        migrate_legacy_ui_state_at(&db, &path);
        assert!(!expand_all(&db));

        // 回退到旧版本又写了这个键（还有别的键）：按旧版最后的值再迁一次，别的键留着
        std::fs::write(&path, r#"{"subtaskExpandAll": true, "other": 1}"#).unwrap();
        migrate_legacy_ui_state_at(&db, &path);
        assert!(expand_all(&db));
        let rest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(rest, serde_json::json!({ "other": 1 }));

        let _ = std::fs::remove_dir_all(&dir);
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
}
