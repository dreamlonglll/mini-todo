//! 待办详情 / 编辑窗口（旧 `EditorView.vue`）。**W2 实现**；现在是占位视图。
//!
//! 规格：`research/ui-editor-windows.md` §2、§3。窗口 1080×600、可缩放、不透明、无系统标题栏，
//! 在打开方所在显示器居中；打开方：主窗口（列表 / 四象限 / 日历 / FAB）、已完成窗口、托盘（无 owner）。

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, IntoElement, Render, SharedString, Window,
};

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size, placeholder};

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

/// 编辑窗口视图（W2 替换内容）
pub struct EditorWindow {
    pub todo_id: Option<i64>,
    pub mode: EditorMode,
}

impl EditorWindow {
    pub fn new(
        todo_id: Option<i64>,
        mode: EditorMode,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self { todo_id, mode }
    }

    /// 系统窗口标题：已有待办 `待办详情`，新建 `新建待办`
    pub fn os_title(todo_id: Option<i64>) -> &'static str {
        if todo_id.is_some() {
            "待办详情"
        } else {
            "新建待办"
        }
    }
}

impl Render for EditorWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let title = match (self.todo_id, self.mode) {
            (None, _) => "新建待办",
            (Some(_), EditorMode::View) => "待办详情",
            (Some(_), EditorMode::Edit) => "编辑待办",
        };
        placeholder(
            "editor-title",
            SharedString::from(title),
            SharedString::from(format!("待办编辑窗口（W2 实现）id = {:?}", self.todo_id)),
        )
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
