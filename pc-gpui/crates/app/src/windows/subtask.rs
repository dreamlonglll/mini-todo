//! 子任务查看 / 编辑窗口（旧 `SubtaskEditorView.vue`）。**W2 实现**；现在是占位视图。
//!
//! 规格：`research/ui-editor-windows.md` §4。窗口 800×750、可缩放、不透明，打开方是编辑窗口（owner）。
//! 新建待办（尚未入库）时的"内存子任务"由 W2 扩展本入口（例如增加 `Entity<DraftSubtasks>` 参数）。

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, IntoElement, Render, SharedString, Window,
};

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size, placeholder};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubtaskMode {
    #[default]
    View,
    Edit,
}

impl SubtaskMode {
    pub fn parse(value: &str) -> Option<SubtaskMode> {
        match value {
            "view" => Some(SubtaskMode::View),
            "edit" => Some(SubtaskMode::Edit),
            _ => None,
        }
    }

    /// 系统窗口标题 / 标题栏文字
    pub fn title(self) -> &'static str {
        match self {
            SubtaskMode::View => "查看子任务",
            SubtaskMode::Edit => "编辑子任务",
        }
    }
}

pub const WIDTH: f32 = 800.;
pub const HEIGHT: f32 = 750.;

/// 子任务窗口视图（W2 替换内容）
pub struct SubtaskWindow {
    pub subtask_id: i64,
    pub mode: SubtaskMode,
}

impl Render for SubtaskWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        placeholder(
            "subtask-title",
            SharedString::from(self.mode.title()),
            SharedString::from(format!("子任务窗口（W2 实现）id = {}", self.subtask_id)),
        )
    }
}

/// 打开子任务窗口（打开方：编辑窗口 `ChildOf::window(window, cx)`；截图调试 `None`）
pub fn open(
    cx: &mut App,
    parent: Option<ChildOf>,
    subtask_id: i64,
    mode: SubtaskMode,
) -> Option<AnyWindowHandle> {
    let spec = WindowSpec::new(
        mode.title(),
        gpui_kit::Bounds::new(Default::default(), logical_size(WIDTH, HEIGHT)),
    );
    WindowManager::open_child(
        cx,
        parent.as_ref(),
        WindowKind::Subtask,
        None,
        spec,
        |_, cx| cx.new(|_| SubtaskWindow { subtask_id, mode }),
    )
}
