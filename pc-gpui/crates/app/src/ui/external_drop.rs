//! 外部文件拖入的放置区（替代 `.on_drop::<ExternalPaths>()`）。
//!
//! ## 为什么不用 GPUI 的 `on_drop`
//!
//! gpui-pre 0.3.9 把系统文件拖放（Windows OLE `IDropTarget`、X11 XDND）转成鼠标事件：`Entered` / `Pending` →
//! `MouseMove`、`Submit` → `MouseUp`。但输入模态（`last_input_modality`）是按**转换前**的 `PlatformInput::FileDrop`
//! 判断的，不会因为文件拖放变成"鼠标"；而 `on_drop` 只在 `hitbox.is_hovered(window)` 时触发，`is_hovered` 在键盘模态下
//! 一律返回 false。于是**窗口里最后一次输入是键盘**（例如收纳箱双击标题改名后按回车、在 Markdown 编辑器里打字）、
//! 鼠标又没有在窗口上移动过（拖放期间系统把鼠标捕获在拖放源进程里，目标窗口收不到 `WM_MOUSEMOVE`）时，
//! 松手的 `Submit` 被静默丢掉：文件没收进来、`drag_over` / 拖入悬停状态残留（`Exited` 也不会来）。
//! GPUI 自己的 `on_file_drop_exit` 用的是忽略模态的 `is_hovered_ignoring_last_input`，`on_drop` 漏了。
//!
//! ## 做法
//!
//! - 放置区元素上 `on_drag_move::<ExternalPaths>`（捕获阶段、不看悬停）记下这次拖入的路径；
//! - 一个铺满放置区的 `canvas` 在绘制阶段注册窗口级监听：`MouseUp`（捕获阶段，最先执行）时有记下的路径、且 GPUI 仍有
//!   活动拖动 → 判断落点（[`DropScope::Window`] 整个窗口都算；[`DropScope::Element`] 用本区域的 hitbox，只看命中、
//!   不看输入模态，被 `occlude()` 的浮层挡住时不算）→ 结束 GPUI 的拖动（它自己的 `on_drop` 不会再重复处理）→ 回调；
//! - `FileDropEvent::Exited`（拖出窗口）同样在窗口级处理，不依赖悬停；
//! - 记下的路径只活在一次拖入里：真实的鼠标按下、或不带按键的真实鼠标移动（文件拖放转成的 `MouseMove` 总是带左键）都清掉它，
//!   万一某次 `Submit` 在平台层丢了，之后用户的普通点击也不会把那批文件当成拖入。
//!
//! 路径存在放置区所属视图持有的 [`ExternalDrop`] 里（跨帧共享：拖动途中重绘后，新一帧的监听仍能拿到）。

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    App, DispatchPhase, ExternalPaths, FileDropEvent, HitboxBehavior, InteractiveElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Styled as _, Window,
    canvas,
};

/// 落点范围
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropScope {
    /// 窗口里任何位置松手都算（收纳箱：旧版 `onDragDropEvent` 是窗口级的）
    Window,
    /// 只有落在放置区上（没有被 `occlude()` 的元素挡住）才算
    Element,
}

/// 一个放置区的拖入状态（在视图构造时创建，每帧 [`ExternalDrop::attach`]）
#[derive(Clone, Default)]
pub struct ExternalDrop {
    pending: Rc<RefCell<Option<ExternalPaths>>>,
}

impl ExternalDrop {
    pub fn new() -> Self {
        Self::default()
    }

    /// 给放置区元素挂上拖入处理：`on_drop` 收到这次拖入的全部路径；`on_exit` 在拖出窗口（`FileDropEvent::Exited`）时调用，
    /// 不需要可传空闭包。两个回调都可以直接用 `cx.listener(..)`
    pub fn attach<E>(
        &self,
        element: E,
        scope: DropScope,
        on_drop: impl Fn(&ExternalPaths, &mut Window, &mut App) + 'static,
        on_exit: impl Fn(&FileDropEvent, &mut Window, &mut App) + 'static,
    ) -> E
    where
        E: InteractiveElement + ParentElement,
    {
        let record = self.pending.clone();
        let pending = self.pending.clone();
        let on_drop = Rc::new(on_drop);
        let on_exit = Rc::new(on_exit);
        let listeners = canvas(
            move |bounds, window, _| {
                (scope == DropScope::Element)
                    .then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal))
            },
            move |_, hitbox, window, _| {
                let up_pending = pending.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                        return;
                    }
                    let Some(paths) = up_pending.borrow_mut().take() else {
                        return;
                    };
                    if !cx.has_active_drag() {
                        return;
                    }
                    // 命中检测不看输入模态（`is_hovered` 在键盘模态下恒为 false，正是要绕开的）
                    if let Some(hitbox) = &hitbox
                        && !hitbox.should_handle_scroll(window)
                    {
                        return;
                    }
                    cx.stop_active_drag(window);
                    cx.stop_propagation();
                    on_drop(&paths, window, cx);
                });
                let exit_pending = pending.clone();
                window.on_mouse_event(move |event: &FileDropEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && matches!(event, FileDropEvent::Exited) {
                        exit_pending.borrow_mut().take();
                        on_exit(event, window, cx);
                    }
                });
                let down_pending = pending.clone();
                window.on_mouse_event(move |_: &MouseDownEvent, phase, _, _| {
                    if phase == DispatchPhase::Capture {
                        down_pending.borrow_mut().take();
                    }
                });
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, _| {
                    if phase == DispatchPhase::Capture && event.pressed_button.is_none() {
                        pending.borrow_mut().take();
                    }
                });
            },
        )
        .absolute()
        .size_full();
        element
            .on_drag_move::<ExternalPaths>(move |event, _, cx| {
                *record.borrow_mut() = Some(event.drag(cx).clone());
            })
            .child(listeners)
    }
}
