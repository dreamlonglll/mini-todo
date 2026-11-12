//! 更新日志窗口：升级后第一次运行时自动弹出（`services::changelog::check_on_startup`，打开方是默认清单窗口、只显示升级区间内的
//! 版本），设置 → 关于「更新日志」打开（打开方是设置窗口，显示全部版本）；`MINI_TODO_DEBUG_OPEN=changelog` 截图用（全部版本）。
//!
//! 520×600、可缩放（最小 420×400）、不透明白底：自绘标题栏「更新日志」（可拖动、右侧关闭）→ 可滚动的内容区（自动弹出时先一句
//! "已更新到 x"，然后每个版本一个标题：版本号 + 日期，内容交给 `ui::markdown` 只读渲染，版本之间一条分隔线）→ 底部「知道了」主按钮。
//! Esc 关闭（复用编辑类窗口的 `util::editor_shortcuts::EditorClose` 绑定）。尺寸 token 在 `theme::changelog_window`。

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
    relative,
};

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size};
use crate::services::changelog::{self, Section};
use crate::state::APP_VERSION;
use crate::theme::{self, changelog_window as dims, light};
use crate::ui::button::EpButton;
use crate::ui::markdown::MarkdownView;
use crate::ui::scrollbar::{self, ScrollbarState};
use crate::ui::title_bar::WindowTitleBar;
use crate::util::editor_shortcuts::{self, EditorClose};

/// 窗口标题（系统标题与自绘标题栏）
pub const TITLE: &str = "更新日志";

/// 更新日志窗口视图
pub struct ChangelogWindow {
    /// 要显示的版本，从新到旧
    sections: Vec<Section>,
    /// 自动弹出：开头显示"已更新到 x"
    upgraded: bool,
    images_dir: Arc<PathBuf>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    scrollbar: ScrollbarState,
}

impl ChangelogWindow {
    fn new(
        sections: Vec<Section>,
        upgraded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        // Esc 要派发到本窗口的 key context
        window.focus(&focus, cx);
        Self {
            sections,
            upgraded,
            images_dir: Arc::new(mini_todo_core::db::paths::images_dir()),
            focus,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
        }
    }

    fn on_close_action(&mut self, _: &EditorClose, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn render_section(&self, index: usize, section: &Section) -> impl IntoElement {
        let palette = light();
        div()
            .flex()
            .flex_col()
            .when(index > 0, |this| {
                this.mt(px(dims::SECTION_GAP))
                    .pt(px(dims::SECTION_GAP))
                    .border_t_1()
                    .border_color(palette.border_light)
            })
            .child(
                div()
                    .mb(px(dims::HEADER_GAP))
                    .flex()
                    .items_baseline()
                    .gap(px(dims::HEADER_GAP))
                    .child(
                        div()
                            .text_size(px(dims::VERSION_SIZE))
                            .font_weight(theme::WEIGHT_SEMIBOLD)
                            .text_color(palette.text_primary)
                            .child(SharedString::from(section.version.clone())),
                    )
                    .when_some(section.date.clone(), |this, date| {
                        this.child(
                            div()
                                .text_size(px(dims::DATE_SIZE))
                                .text_color(palette.text_tertiary)
                                .child(date),
                        )
                    }),
            )
            .child(MarkdownView::new(
                SharedString::from(format!("changelog-{index}")),
                SharedString::from(section.body.clone()),
                self.images_dir.clone(),
            ))
    }
}

impl Focusable for ChangelogWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ChangelogWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let intro = self.upgraded.then(|| {
            div()
                .mb(px(dims::SECTION_GAP))
                .text_size(px(dims::INTRO_SIZE))
                .text_color(palette.text_secondary)
                .child(format!(
                    "Mini Todo 已更新到 {APP_VERSION}，以下是新版本的变化。"
                ))
        });
        let sections: Vec<gpui_kit::AnyElement> = self
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| self.render_section(index, section).into_any_element())
            .collect();
        let empty = self.sections.is_empty().then(|| {
            div()
                .text_size(px(dims::INTRO_SIZE))
                .text_color(palette.text_tertiary)
                .child("暂无更新日志")
        });
        let content = div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(
                div()
                    .id("changelog-content")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .px(px(dims::PADDING_X))
                            .py(px(dims::PADDING_Y))
                            .flex()
                            .flex_col()
                            .children(intro)
                            .children(sections)
                            .children(empty),
                    ),
            )
            .child(scrollbar::vertical(
                "changelog-scrollbar",
                &self.scroll,
                &self.scrollbar,
                dims::SCROLLBAR,
                palette.border,
                palette.text_tertiary,
            ));
        let footer = div()
            .flex_none()
            .flex()
            .justify_end()
            .px(px(dims::FOOTER_PADDING_X))
            .py(px(dims::FOOTER_PADDING_Y))
            .bg(palette.bg_secondary)
            .border_t_1()
            .border_color(palette.border)
            .child(
                EpButton::new("changelog-ok")
                    .primary()
                    .label("知道了")
                    .on_click(|_, window, _| window.remove_window()),
            );
        div()
            .id("changelog-window")
            .key_context(editor_shortcuts::EDITOR_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_close_action))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .text_color(palette.text_primary)
            .bg(palette.window_bg)
            .child(WindowTitleBar::new("changelog-title", TITLE))
            .child(content)
            .child(footer)
    }
}

/// 打开更新日志窗口，显示 `sections`（从新到旧）。`upgraded`：升级后自动弹出（开头说明"已更新到 x"）。
/// `parent`：打开方（自动弹出：默认清单窗口 `ChildOf::handle(.., cx).unowned()`；关于页：设置窗口 `ChildOf::window`；截图调试 `None`）。
/// 打开方已有模态子窗口时只把它带到前台、返回 `None`
pub fn open(
    cx: &mut App,
    parent: Option<ChildOf>,
    sections: Vec<Section>,
    upgraded: bool,
) -> Option<AnyWindowHandle> {
    editor_shortcuts::init(cx);
    let mut spec = WindowSpec::new(
        TITLE,
        gpui_kit::Bounds::new(Default::default(), logical_size(dims::WIDTH, dims::HEIGHT)),
    );
    spec.min_size = Some(logical_size(dims::MIN_WIDTH, dims::MIN_HEIGHT));
    WindowManager::open_child(
        cx,
        parent.as_ref(),
        WindowKind::Changelog,
        None,
        spec,
        move |window, cx| cx.new(|cx| ChangelogWindow::new(sections, upgraded, window, cx)),
    )
}

/// 打开显示全部版本（`<=` 当前版本，不含「未发布」）的更新日志窗口（设置 → 关于、`MINI_TODO_DEBUG_OPEN=changelog`）
pub fn open_all(cx: &mut App, parent: Option<ChildOf>) -> Option<AnyWindowHandle> {
    let sections = changelog::sections_between(changelog::sections(), None, APP_VERSION);
    open(cx, parent, sections, false)
}
