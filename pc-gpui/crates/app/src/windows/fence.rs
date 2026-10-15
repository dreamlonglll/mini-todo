//! 桌面收纳箱窗口（旧 `FenceView.vue`，label `fence-{id}`）。**W3 实现**；现在是占位视图。
//!
//! 规格：`research/ui-settings-fence.md` §3、§4，契约 `.trellis/spec/backend/desktop-fences.md`。
//! 无边框透明、不可缩放 / 最小化、`show: false` 建窗 → 嵌入桌面（`platform::embed`）→ 显示；拦截关闭
//! （`window.on_window_should_close(cx, |_, _| false)`），删除收纳箱时 `remove_window()`。
//! 几何是物理像素（`mini_todo_core::fence::fence_view` + `platform::set_window_rect_physical`）。

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Window, div, px,
};

use super::{WindowKind, WindowManager, WindowSpec, logical_size};
use crate::theme;

/// 窗口登记用的 key（与旧版 label 相同：`fence-{id}`）
pub fn fence_key(fence_id: i64) -> String {
    mini_todo_core::fence::fence_label(fence_id)
}

/// 收纳箱窗口视图（W3 替换内容）
pub struct FenceWindow {
    pub fence_id: i64,
}

impl Render for FenceWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .font(theme::ui_font())
            .text_size(px(13.))
            .text_color(theme::hex(0xFFFFFF))
            .bg(theme::rgba_css(0, 0, 0, 0.45))
            .border_1()
            .border_color(theme::rgba_css(255, 255, 255, 0.14))
            .rounded(theme::radius::MD)
            .child(
                div()
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .child(SharedString::from(format!("收纳箱 {}", self.fence_id))),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::rgba_css(255, 255, 255, 0.55))
                    .child("收纳箱窗口（W3 实现）"),
            )
    }
}

/// 启动时恢复全部收纳箱窗口（`run()` 里调用；旧 `restore_fences`：`fence::prepare_restore` → 逐个建窗 →
/// 有收纳箱则 `fence::items::watch` + `spawn_startup_reconcile`）。**W3 实现**（只在 Windows 上恢复）；现在什么也不做。
pub fn restore_all(_cx: &mut App) {}

/// 打开（或带到前台）某个收纳箱窗口
pub fn open(cx: &mut App, fence_id: i64) -> Option<AnyWindowHandle> {
    let key = fence_key(fence_id);
    if let Some(existing) = WindowManager::find(cx, WindowKind::Fence, &key) {
        return Some(existing);
    }
    let mut spec = WindowSpec::new(
        format!("收纳箱 {fence_id}"),
        Bounds::new(Default::default(), logical_size(368., 336.)),
    );
    spec.transparent = true;
    spec.resizable = false;
    spec.minimizable = false;
    spec.focus = false;
    WindowManager::open_child(
        cx,
        None,
        WindowKind::Fence,
        Some(key),
        spec,
        move |_, cx| cx.new(|_| FenceWindow { fence_id }),
    )
}
