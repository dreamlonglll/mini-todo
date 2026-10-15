//! 窗口管理：按种类打开 / 聚焦窗口、记录句柄、模态子窗口（旧版 `isModalOpen`）、子窗口关闭回调、
//! 在打开方所在显示器居中、统一的窗口参数。
//!
//! | 文件 | 窗口 | 负责 |
//! |---|---|---|
//! | `main_window.rs` | 主窗口（透明、无系统标题栏） | W1（现在是能显示待办标题的简版） |
//! | `editor.rs` | 待办详情 / 编辑 | W2 |
//! | `subtask.rs` | 子任务查看 / 编辑 | W2 |
//! | `completed.rs` | 已完成 | W2 |
//! | `notification.rs` | 应用内通知（透明、置顶、不抢焦点） | W2 |
//! | `settings.rs` | 设置 | W3 |
//! | `fence.rs` | 桌面收纳箱（透明、嵌入桌面） | W3 |
//!
//! ## 模态子窗口（旧版 `isModalOpen` 语义）
//!
//! 每个打开方（主窗口、已完成窗口、编辑窗口）同一时刻只允许一个子窗口：再请求时直接把已有子窗口带到前台、
//! 不再打开。打开期间打开方显示 [`modal_overlay`]（满窗遮罩，点击把子窗口带到前台），打开方获得焦点时
//! 调 [`WindowManager::forward_activation`] 把焦点交给子窗口。子窗口关闭后 [`WindowManager`] 全局变化
//! （打开方 `cx.observe_global::<WindowManager>` 重绘），并执行打开时登记的关闭回调（刷新数据等）。
//!
//! Windows 上子窗口以打开方为 owner（始终在打开方之上、不单独占任务栏按钮），与旧版 Tauri `parent` 一致；
//! 托盘「添加待办项」打开的编辑窗口仍以主窗口为模态打开方，但没有 owner（[`ChildOf::unowned`]）。

pub mod completed;
pub mod editor;
pub mod fence;
pub mod gallery;
pub mod main_window;
pub mod notification;
pub mod settings;
pub mod subtask;

use std::collections::HashMap;

use gpui_kit::base::Root;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, Bounds, CursorStyle,
    DisplayId, Entity, Global, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Pixels, Render, SharedString, Size, Styled as _, Subscription,
    TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowHandle, WindowId, WindowKind as GpuiWindowKind, WindowOptions, div, point, px, size,
    transparent_black,
};

use crate::platform::{self, Hwnd, MonitorInfo};
use crate::theme::{self, light};
use crate::ui::title_bar::WindowTitleBar;

/// 窗口种类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowKind {
    Main,
    Editor,
    Subtask,
    Settings,
    Completed,
    Notification,
    Fence,
    /// 调试用的控件样例窗口（`MINI_TODO_DEBUG_OPEN=gallery`）
    Gallery,
}

/// 子窗口的打开方（模态追踪 + 居中 + owner）
#[derive(Clone, Debug)]
pub struct ChildOf {
    /// 模态追踪的打开方（它会显示遮罩）
    pub opener: AnyWindowHandle,
    /// 是否以打开方为 owner（Windows：始终在其上、不单独占任务栏）
    pub owned: bool,
    monitor: Option<MonitorInfo>,
    opener_hwnd: Option<Hwnd>,
    opener_bounds: Option<Bounds<Pixels>>,
}

impl ChildOf {
    /// 在打开方自己的事件处理里用（打开方窗口正被借用，不能再 `update_window` 它）
    pub fn window(window: &Window, cx: &App) -> Self {
        Self {
            opener: window.window_handle(),
            owned: true,
            monitor: platform::monitor_of_window(window, cx),
            opener_hwnd: platform::hwnd(window),
            opener_bounds: Some(window.bounds()),
        }
    }

    /// 只有句柄时（托盘、后台任务）；打开方正被借用时取不到显示器，按主显示器居中
    pub fn handle(opener: AnyWindowHandle, cx: &mut App) -> Self {
        let info = cx
            .update_window(opener, |_, window, cx| {
                (
                    platform::monitor_of_window(window, cx),
                    platform::hwnd(window),
                    window.bounds(),
                )
            })
            .ok();
        Self {
            opener,
            owned: true,
            monitor: info.as_ref().and_then(|(m, _, _)| m.clone()),
            opener_hwnd: info.as_ref().and_then(|(_, h, _)| *h),
            opener_bounds: info.map(|(_, _, b)| b),
        }
    }

    /// 不设 owner（托盘新建待办：独立顶层窗口，会出现在任务栏）
    pub fn unowned(mut self) -> Self {
        self.owned = false;
        self
    }
}

/// 在显示器（整块，不是工作区）上居中的逻辑矩形：`x = round(mon.x/s + (mon.w/s − W)/2)`，y 同理；
/// 没有显示器信息时以打开方窗口为中心
pub fn centered_bounds(
    monitor: Option<&MonitorInfo>,
    opener_bounds: Option<Bounds<Pixels>>,
    window_size: Size<Pixels>,
) -> Bounds<Pixels> {
    let (w, h) = (f32::from(window_size.width), f32::from(window_size.height));
    let (cx, cy, cw, ch) = match (monitor, opener_bounds) {
        (Some(m), _) => {
            let r = m.logical_rect();
            (
                f32::from(r.origin.x),
                f32::from(r.origin.y),
                f32::from(r.size.width),
                f32::from(r.size.height),
            )
        }
        (None, Some(b)) => (
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
        ),
        (None, None) => (0., 0., w, h),
    };
    Bounds::new(
        point(
            px((cx + (cw - w) / 2.).round()),
            px((cy + (ch - h) / 2.).round()),
        ),
        window_size,
    )
}

/// 统一的窗口参数
#[derive(Clone, Debug)]
pub struct WindowSpec {
    /// 系统窗口标题（任务栏 / Alt+Tab）
    pub title: SharedString,
    pub bounds: Bounds<Pixels>,
    pub display_id: Option<DisplayId>,
    pub resizable: bool,
    pub minimizable: bool,
    /// 透明窗口（主窗口 / 通知 / 收纳箱）：Root 背景覆盖为透明，界面自己画圆角底
    pub transparent: bool,
    pub focus: bool,
    pub show: bool,
    pub kind: GpuiWindowKind,
    pub min_size: Option<Size<Pixels>>,
}

impl WindowSpec {
    pub fn new(title: impl Into<SharedString>, bounds: Bounds<Pixels>) -> Self {
        Self {
            title: title.into(),
            bounds,
            display_id: None,
            resizable: true,
            minimizable: true,
            transparent: false,
            focus: true,
            show: true,
            kind: GpuiWindowKind::Normal,
            min_size: None,
        }
    }

    /// 无系统标题栏（`decorations: false`）的 GPUI 窗口参数
    pub fn options(&self) -> WindowOptions {
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(self.bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(self.title.clone()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            focus: self.focus,
            show: self.show,
            kind: self.kind.clone(),
            is_movable: true,
            is_resizable: self.resizable,
            is_minimizable: self.minimizable,
            display_id: self.display_id,
            window_background: if self.transparent {
                WindowBackgroundAppearance::Transparent
            } else {
                WindowBackgroundAppearance::Opaque
            },
            app_id: Some("mini-todo".into()),
            window_min_size: self.min_size,
            window_decorations: Some(WindowDecorations::Client),
            ..Default::default()
        }
    }
}

/// 打开一个窗口（根视图包一层 `Root`，透明窗口把 Root 背景覆盖为透明）
pub fn open_window<V: Render>(
    cx: &mut App,
    spec: &WindowSpec,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> gpui_kit::Result<WindowHandle<Root>> {
    let transparent = spec.transparent;
    cx.open_window(spec.options(), move |window, cx| {
        let view = build(window, cx);
        cx.new(|cx| {
            let root = Root::new(view, window, cx);
            if transparent {
                root.bg(transparent_black())
            } else {
                root
            }
        })
    })
}

struct Registered {
    kind: WindowKind,
    key: Option<String>,
    handle: AnyWindowHandle,
}

type CloseCallback = Box<dyn FnOnce(&mut App)>;

/// 全部窗口与模态关系（Global）
#[derive(Default)]
pub struct WindowManager {
    main: Option<AnyWindowHandle>,
    windows: HashMap<WindowId, Registered>,
    /// 打开方 → 它的模态子窗口
    modal_children: HashMap<WindowId, AnyWindowHandle>,
    on_closed: HashMap<WindowId, Vec<CloseCallback>>,
    _closed: Option<Subscription>,
}

impl Global for WindowManager {}

impl WindowManager {
    /// `main` 里调用一次
    pub fn init(cx: &mut App) {
        let subscription = cx.on_window_closed(WindowManager::handle_closed);
        cx.set_global(WindowManager {
            _closed: Some(subscription),
            ..Default::default()
        });
    }

    fn handle_closed(cx: &mut App, id: WindowId) {
        let callbacks = cx.update_global::<WindowManager, _>(|manager, _| {
            manager.windows.remove(&id);
            if manager.main.is_some_and(|m| m.window_id() == id) {
                manager.main = None;
            }
            manager
                .modal_children
                .retain(|_, child| child.window_id() != id);
            manager.modal_children.remove(&id);
            manager.on_closed.remove(&id).unwrap_or_default()
        });
        for callback in callbacks {
            callback(cx);
        }
    }

    /// 登记窗口（`key`：收纳箱 `fence-{id}`、通知 `notification_{n}` 等，可为空）
    pub fn register(cx: &mut App, kind: WindowKind, key: Option<String>, handle: AnyWindowHandle) {
        cx.update_global::<WindowManager, _>(|manager, _| {
            if kind == WindowKind::Main {
                manager.main = Some(handle);
            }
            manager
                .windows
                .insert(handle.window_id(), Registered { kind, key, handle });
        });
    }

    /// 窗口关闭后执行一次（打开方刷新数据等）
    pub fn on_closed(
        cx: &mut App,
        handle: AnyWindowHandle,
        callback: impl FnOnce(&mut App) + 'static,
    ) {
        cx.update_global::<WindowManager, _>(|manager, _| {
            manager
                .on_closed
                .entry(handle.window_id())
                .or_default()
                .push(Box::new(callback));
        });
    }

    pub fn main(cx: &App) -> Option<AnyWindowHandle> {
        cx.try_global::<WindowManager>().and_then(|m| m.main)
    }

    pub fn kind_of(cx: &App, id: WindowId) -> Option<WindowKind> {
        cx.try_global::<WindowManager>()?
            .windows
            .get(&id)
            .map(|r| r.kind)
    }

    /// 某种类的全部窗口
    pub fn all(cx: &App, kind: WindowKind) -> Vec<AnyWindowHandle> {
        cx.try_global::<WindowManager>()
            .map(|m| {
                m.windows
                    .values()
                    .filter(|r| r.kind == kind)
                    .map(|r| r.handle)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 按 key 找窗口
    pub fn find(cx: &App, kind: WindowKind, key: &str) -> Option<AnyWindowHandle> {
        cx.try_global::<WindowManager>()?
            .windows
            .values()
            .find(|r| r.kind == kind && r.key.as_deref() == Some(key))
            .map(|r| r.handle)
    }

    /// 打开方当前的模态子窗口
    pub fn modal_child(cx: &App, opener: WindowId) -> Option<AnyWindowHandle> {
        cx.try_global::<WindowManager>()?
            .modal_children
            .get(&opener)
            .copied()
    }

    pub fn has_modal_child(cx: &App, opener: WindowId) -> bool {
        Self::modal_child(cx, opener).is_some()
    }

    /// 主窗口是否有模态子窗口（5s 轮询、同步后刷新据此跳过）
    pub fn main_has_modal_child(cx: &App) -> bool {
        Self::main(cx).is_some_and(|main| Self::has_modal_child(cx, main.window_id()))
    }

    /// 把打开方的模态子窗口带到前台；没有返回 false
    pub fn focus_modal_child(cx: &mut App, opener: WindowId) -> bool {
        let Some(child) = Self::modal_child(cx, opener) else {
            return false;
        };
        cx.update_window(child, |_, window, cx| platform::bring_to_front(window, cx))
            .is_ok()
    }

    /// 打开方获得焦点时调用（`cx.observe_window_activation` 里）：有模态子窗口就把焦点交给它，返回 true
    pub fn forward_activation(window: &Window, cx: &mut App) -> bool {
        if !window.is_window_active() {
            return false;
        }
        Self::focus_modal_child(cx, window.window_handle().window_id())
    }

    /// 打开子窗口（`parent` 为 `None` 时是独立窗口）：
    /// - 打开方已有模态子窗口 → 带到前台，返回 `None`
    /// - 在打开方所在显示器居中（`spec.bounds` 只用其尺寸）、登记、设 owner、记录模态关系
    /// - 打开方是主窗口时，子窗口关闭后自动 `AppState::reload_todos`；设置窗口另外重读设置并重建自动同步
    ///   （旧 MainView 的 `tauri://destroyed` 处理）。其它刷新用 [`Self::on_closed`] 自己登记
    pub fn open_child<V: Render>(
        cx: &mut App,
        parent: Option<&ChildOf>,
        kind: WindowKind,
        key: Option<String>,
        mut spec: WindowSpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Option<AnyWindowHandle> {
        if let Some(parent) = parent
            && Self::focus_modal_child(cx, parent.opener.window_id())
        {
            return None;
        }
        let monitor = match parent {
            Some(p) => p.monitor.clone(),
            None => platform::primary_monitor(cx),
        };
        spec.bounds = centered_bounds(
            monitor.as_ref(),
            parent.and_then(|p| p.opener_bounds),
            spec.bounds.size,
        );
        spec.display_id = monitor.as_ref().map(|m| m.display_id);
        let handle: AnyWindowHandle = match open_window(cx, &spec, build) {
            Ok(handle) => handle.into(),
            Err(e) => {
                log::error!("[window] 打开 {kind:?} 窗口失败: {e}");
                return None;
            }
        };
        Self::register(cx, kind, key, handle);
        if let Some(parent) = parent {
            let opener_id = parent.opener.window_id();
            cx.update_global::<WindowManager, _>(|manager, _| {
                manager.modal_children.insert(opener_id, handle);
            });
            if parent.owned
                && let Some(owner) = parent.opener_hwnd
            {
                let _ = cx.update_window(handle, |_, window, cx| {
                    platform::set_owner(window, cx, Some(owner));
                });
            }
            // 旧 MainView：主窗口的子窗口销毁后 fetchTodos()；设置窗口另外 reloadAppSettings() + startAutoSync()
            if Self::main(cx).is_some_and(|main| main.window_id() == opener_id) {
                Self::on_closed(cx, handle, move |cx| {
                    let state = crate::state::AppState::global(cx);
                    state.update(cx, |state, cx| {
                        state.reload_todos(cx);
                        if kind == WindowKind::Settings {
                            state.reload_app_settings(cx).detach();
                        }
                    });
                    if kind == WindowKind::Settings {
                        crate::services::auto_sync::start(cx);
                    }
                });
            }
        }
        Some(handle)
    }

    /// 关闭窗口
    pub fn close(cx: &mut App, handle: AnyWindowHandle) {
        let _ = cx.update_window(handle, |_, window, _| window.remove_window());
    }
}

/// 打开方的模态遮罩（旧版 `.modal-overlay`）：满窗 `rgba(0,0,0,alpha)`（主窗口 / 已完成 0.3、编辑窗口 0.15），
/// `cursor: not-allowed`，按下把子窗口带到前台。打开方 render 的**最后一个**子元素；没有模态子窗口时返回 `None`。
pub fn modal_overlay(window: &Window, cx: &App, alpha: f32) -> Option<AnyElement> {
    let opener = window.window_handle().window_id();
    WindowManager::has_modal_child(cx, opener).then(|| {
        div()
            .id("modal-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme::rgba_css(0, 0, 0, alpha))
            .cursor(CursorStyle::OperationNotAllowed)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                WindowManager::focus_modal_child(cx, opener);
            })
            .into_any_element()
    })
}

/// 占位窗口内容（W2 / W3 替换）：标题栏 + 窗口名 + 说明
pub(crate) fn placeholder(
    id: &'static str,
    title: SharedString,
    note: SharedString,
) -> impl IntoElement {
    let palette = light();
    div()
        .size_full()
        .flex()
        .flex_col()
        .font(theme::ui_font())
        .text_size(theme::FONT_SIZE)
        .line_height(gpui_kit::relative(theme::LINE_HEIGHT))
        .bg(theme::hex(0xFFFFFF))
        .text_color(palette.text_primary)
        .child(WindowTitleBar::new(id, title.clone()))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(div().text_size(px(18.)).child(title))
                .child(div().text_color(palette.text_tertiary).child(note)),
        )
}

/// 逻辑尺寸
pub fn logical_size(width: f32, height: f32) -> Size<Pixels> {
    size(px(width), px(height))
}

// ----------------------------------------------------------------------
// MINI_TODO_DEBUG_OPEN（仅 debug 构建）：启动后额外打开指定窗口，供截图脚本使用
// ----------------------------------------------------------------------

/// `MINI_TODO_DEBUG_OPEN` 里的一项
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugOpen {
    Main,
    Editor {
        todo_id: Option<i64>,
        mode: editor::EditorMode,
    },
    Subtask {
        subtask_id: i64,
        mode: subtask::SubtaskMode,
    },
    Settings(settings::SettingsTab),
    Completed,
    Notification {
        todo_id: Option<i64>,
    },
    Fence(i64),
    /// 在主窗口里弹一条 ElMessage（`toast:<success|info|warning|error>:<文字>`）
    Toast(crate::ui::message::MessageKind, String),
    /// 在主窗口里弹 ElMessageBox.confirm（`confirm:<标题>:<内容>`，warning 图标、`确定` / `取消`）
    Confirm(String, String),
    /// 控件样例窗口（`gallery`）
    Gallery,
    /// 运行时打开日历（`calendar`，W1：与设置里打开「展示日历」相同，主窗口按公式加宽）
    Calendar,
    /// 新建一个收纳箱（`fence:new`，与托盘「新增收纳箱」相同；W3）
    NewFence,
}

/// 解析 `main,editor:1:view,editor:new,subtask:3:edit,settings:general,completed,notification:1,fence:1`；
/// 无法识别的项记 warning 并跳过
pub fn parse_debug_open(spec: &str) -> Vec<DebugOpen> {
    spec.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|item| {
            let parts: Vec<&str> = item.split(':').map(str::trim).collect();
            let parsed = match parts.as_slice() {
                ["main"] => Some(DebugOpen::Main),
                ["editor"] | ["editor", "new"] => Some(DebugOpen::Editor {
                    todo_id: None,
                    mode: editor::EditorMode::Edit,
                }),
                ["editor", id] => id.parse().ok().map(|id| DebugOpen::Editor {
                    todo_id: Some(id),
                    mode: editor::EditorMode::View,
                }),
                ["editor", id, mode] => {
                    let mode = editor::EditorMode::parse(mode)?;
                    id.parse().ok().map(|id| DebugOpen::Editor {
                        todo_id: Some(id),
                        mode,
                    })
                }
                ["subtask", id] => id.parse().ok().map(|subtask_id| DebugOpen::Subtask {
                    subtask_id,
                    mode: subtask::SubtaskMode::View,
                }),
                ["subtask", id, mode] => {
                    let mode = subtask::SubtaskMode::parse(mode)?;
                    id.parse()
                        .ok()
                        .map(|subtask_id| DebugOpen::Subtask { subtask_id, mode })
                }
                ["settings"] => Some(DebugOpen::Settings(settings::SettingsTab::General)),
                ["settings", tab] => settings::SettingsTab::parse(tab).map(DebugOpen::Settings),
                ["completed"] => Some(DebugOpen::Completed),
                ["notification"] => Some(DebugOpen::Notification { todo_id: None }),
                ["notification", id] => id
                    .parse()
                    .ok()
                    .map(|id| DebugOpen::Notification { todo_id: Some(id) }),
                ["fence", "new"] => Some(DebugOpen::NewFence),
                ["fence", id] => id.parse().ok().map(DebugOpen::Fence),
                ["gallery"] => Some(DebugOpen::Gallery),
                ["calendar"] => Some(DebugOpen::Calendar),
                ["toast", kind, text @ ..] if !text.is_empty() => {
                    use crate::ui::message::MessageKind;
                    let kind = match *kind {
                        "success" => Some(MessageKind::Success),
                        "info" => Some(MessageKind::Info),
                        "warning" => Some(MessageKind::Warning),
                        "error" => Some(MessageKind::Error),
                        _ => None,
                    };
                    kind.map(|kind| DebugOpen::Toast(kind, text.join(":")))
                }
                ["confirm", title, message @ ..] if !message.is_empty() => {
                    Some(DebugOpen::Confirm(title.to_string(), message.join(":")))
                }
                _ => None,
            };
            if parsed.is_none() {
                log::warn!("[debug] 无法识别的 MINI_TODO_DEBUG_OPEN 项: {item}");
            }
            parsed
        })
        .collect()
}

/// 打开 `MINI_TODO_DEBUG_OPEN` 指定的窗口（不走模态关系，主窗口不显示遮罩，便于截图）
pub fn open_debug(cx: &mut App, items: &[DebugOpen]) {
    for item in items {
        log::info!("[debug] MINI_TODO_DEBUG_OPEN 打开 {item:?}");
        match item {
            DebugOpen::Main => {
                if WindowManager::main(cx).is_none() {
                    main_window::open(cx);
                }
            }
            DebugOpen::Editor { todo_id, mode } => {
                editor::open(cx, None, *todo_id, *mode);
            }
            DebugOpen::Subtask { subtask_id, mode } => {
                subtask::open(cx, None, *subtask_id, *mode);
            }
            DebugOpen::Settings(tab) => {
                settings::open(cx, None, *tab);
            }
            DebugOpen::Completed => {
                completed::open(cx, None);
            }
            DebugOpen::Notification { todo_id } => {
                let todo = todo_id.and_then(|id| {
                    crate::state::AppState::global(cx)
                        .read(cx)
                        .todo(id)
                        .cloned()
                });
                let content = match todo {
                    Some(todo) => notification::NotificationContent {
                        todo_id: Some(todo.id),
                        title: todo.title,
                        description: todo.description.unwrap_or_default(),
                    },
                    None => notification::NotificationContent {
                        todo_id: *todo_id,
                        title: "待办提醒".into(),
                        description: mini_todo_core::reminder::DEFAULT_BODY.into(),
                    },
                };
                notification::open(cx, content);
            }
            DebugOpen::Fence(id) => {
                fence::open(cx, *id);
            }
            DebugOpen::Gallery => {
                gallery::open(cx);
            }
            DebugOpen::Calendar => {
                crate::state::AppState::global(cx)
                    .update(cx, |state, cx| state.set_show_calendar(true, cx))
                    .detach();
            }
            DebugOpen::NewFence => {
                fence::tray_create_fence(cx);
            }
            DebugOpen::Toast(kind, text) => {
                if let Some(main) = WindowManager::main(cx) {
                    crate::ui::message::show_in(main, cx, *kind, text.clone());
                }
            }
            DebugOpen::Confirm(title, message) => {
                if let Some(main) = WindowManager::main(cx) {
                    let options =
                        crate::ui::message_box::ConfirmOptions::new(title.clone(), message.clone());
                    let _ = cx.update_window(main, |_, window, cx| {
                        crate::ui::message_box::confirm(
                            window,
                            cx,
                            options,
                            |confirmed, window, cx| {
                                log::info!("[debug] 确认框结果: {confirmed}");
                                crate::ui::message::info(
                                    window,
                                    cx,
                                    if confirmed { "已确认" } else { "已取消" },
                                );
                            },
                        );
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_open_spec_parsing() {
        let items = parse_debug_open(
            "main, editor:1:view,editor:new,editor:5:edit,subtask:3:edit,subtask:4,settings:general,settings,completed,notification:1,notification,fence:2,bogus,editor:x",
        );
        assert_eq!(
            items,
            vec![
                DebugOpen::Main,
                DebugOpen::Editor {
                    todo_id: Some(1),
                    mode: editor::EditorMode::View
                },
                DebugOpen::Editor {
                    todo_id: None,
                    mode: editor::EditorMode::Edit
                },
                DebugOpen::Editor {
                    todo_id: Some(5),
                    mode: editor::EditorMode::Edit
                },
                DebugOpen::Subtask {
                    subtask_id: 3,
                    mode: subtask::SubtaskMode::Edit
                },
                DebugOpen::Subtask {
                    subtask_id: 4,
                    mode: subtask::SubtaskMode::View
                },
                DebugOpen::Settings(settings::SettingsTab::General),
                DebugOpen::Settings(settings::SettingsTab::General),
                DebugOpen::Completed,
                DebugOpen::Notification { todo_id: Some(1) },
                DebugOpen::Notification { todo_id: None },
                DebugOpen::Fence(2),
            ]
        );
        assert_eq!(
            parse_debug_open("settings:data"),
            vec![DebugOpen::Settings(settings::SettingsTab::Data)]
        );
        assert_eq!(
            parse_debug_open("fence:new,fence:3"),
            vec![DebugOpen::NewFence, DebugOpen::Fence(3)]
        );
        assert_eq!(
            parse_debug_open(
                "toast:error:同步失败：网络错误,confirm:删除确认:确定要删除待办\"a\"吗？,toast:bad:x"
            ),
            vec![
                DebugOpen::Toast(
                    crate::ui::message::MessageKind::Error,
                    "同步失败：网络错误".into()
                ),
                DebugOpen::Confirm("删除确认".into(), "确定要删除待办\"a\"吗？".into()),
            ]
        );
    }

    #[test]
    fn centered_on_whole_monitor() {
        let monitor = MonitorInfo {
            display_id: DisplayId::new(1),
            rect: platform::PhysRect::new(0, 0, 2880, 1620),
            work: platform::PhysRect::new(0, 0, 2880, 1560),
            scale: 1.5,
            primary: true,
        };
        // 逻辑 1920×1080，1080×600 居中 → (420, 240)
        let bounds = centered_bounds(Some(&monitor), None, logical_size(1080., 600.));
        assert_eq!(bounds.origin, point(px(420.), px(240.)));
        // 没有显示器：以打开方为中心
        let opener = Bounds::new(point(px(100.), px(100.)), logical_size(380., 600.));
        let bounds = centered_bounds(None, Some(opener), logical_size(680., 560.));
        assert_eq!(bounds.origin, point(px(-50.), px(120.)));
    }
}
