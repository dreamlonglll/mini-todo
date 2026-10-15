//! 已完成窗口（旧 `CompletedView.vue`）。**W2 实现**；现在是占位视图。
//!
//! 规格：`research/ui-editor-windows.md` §10。窗口 460×550、可缩放、不透明，打开方是主窗口（owner）；
//! 从这里点开的编辑窗口以本窗口为打开方（遮罩 0.3）。

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, IntoElement, Render, SharedString, Window,
};

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size, placeholder};

pub const WIDTH: f32 = 460.;
pub const HEIGHT: f32 = 550.;

/// 已完成窗口视图（W2 替换内容）
pub struct CompletedWindow;

impl Render for CompletedWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        placeholder(
            "completed-title",
            SharedString::from("已完成"),
            SharedString::from("已完成窗口（W2 实现）"),
        )
    }
}

/// 打开已完成窗口（打开方：主窗口 `ChildOf::window(window, cx)`；截图调试 `None`）
pub fn open(cx: &mut App, parent: Option<ChildOf>) -> Option<AnyWindowHandle> {
    let spec = WindowSpec::new(
        "已完成",
        gpui_kit::Bounds::new(Default::default(), logical_size(WIDTH, HEIGHT)),
    );
    WindowManager::open_child(
        cx,
        parent.as_ref(),
        WindowKind::Completed,
        None,
        spec,
        |_, cx| cx.new(|_| CompletedWindow),
    )
}
