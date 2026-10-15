//! 设置窗口（旧 `SettingsView.vue`），规格 `research/ui-settings-fence.md` §2。
//!
//! 680×560、**不可缩放**、不透明白底，打开方是主窗口（owner + 模态遮罩，由 WindowManager 处理）；入口：标题栏齿轮、
//! 托盘「打开设置」。关闭后 WindowManager 让主窗口刷新待办、重读设置、按新设置重建自动同步。
//!
//! 布局：自绘标题栏（`设置` 17px/600，右侧 text 关闭按钮，可拖动）→ 左侧 160px 菜单（常规 / 外观 / 数据与同步 /
//! 屏幕配置 / 关于）→ 右侧内容区（`padding 20px 24px`、可滚动、底 `#F8FAFC`）。五个面板的状态都保存在本视图里，
//! 切换菜单不丢失（旧版 `v-show`）。
//!
//! 与旧版的差异：设置窗口与主窗口同进程，所有设置直接改 [`AppState`]（替代 `app-settings-changed` /
//! `todo-font-changed` / `data-imported` 事件），主窗口观察它刷新。旧版三个缺陷按预期行为修好（§2.13）：
//! 屏幕配置标出「当前」并禁止删除；关于页直接显示版本号；清空字体下拉 = 恢复默认字体。

mod about;
mod appearance;
mod data;
mod general;
mod screen;
mod update;

use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Div, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, point,
    prelude::FluentBuilder as _, px, relative,
};
use mini_todo_core::sync::SyncReport;

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size};
use crate::assets::EpIcon;
use crate::state::AppState;
use crate::theme::{self, settings_tokens};
use crate::ui::ep_color_picker::{EpColorPicker, EpColorPickerEvent};
use crate::ui::ep_select::{EpSelect, EpSelectEvent, EpSelectOption};
use crate::ui::icon::{icon, spinning};
use crate::ui::input::InputEvent;
use crate::ui::title_bar::WindowTitleBar;

/// 左侧菜单
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    /// 常规
    #[default]
    General,
    /// 外观
    Appearance,
    /// 数据与同步
    Data,
    /// 屏幕配置
    Screen,
    /// 关于
    About,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 5] = [
        SettingsTab::General,
        SettingsTab::Appearance,
        SettingsTab::Data,
        SettingsTab::Screen,
        SettingsTab::About,
    ];

    pub fn parse(value: &str) -> Option<SettingsTab> {
        match value {
            "general" => Some(SettingsTab::General),
            "appearance" => Some(SettingsTab::Appearance),
            "data" => Some(SettingsTab::Data),
            "screen" => Some(SettingsTab::Screen),
            "about" => Some(SettingsTab::About),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            SettingsTab::General => "general",
            SettingsTab::Appearance => "appearance",
            SettingsTab::Data => "data",
            SettingsTab::Screen => "screen",
            SettingsTab::About => "about",
        }
    }

    /// 菜单文案
    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "常规",
            SettingsTab::Appearance => "外观",
            SettingsTab::Data => "数据与同步",
            SettingsTab::Screen => "屏幕配置",
            SettingsTab::About => "关于",
        }
    }

    /// 菜单图标
    pub fn icon(self) -> EpIcon {
        match self {
            SettingsTab::General => EpIcon::Setting,
            SettingsTab::Appearance => EpIcon::Brush,
            SettingsTab::Data => EpIcon::Folder,
            SettingsTab::Screen => EpIcon::Monitor,
            SettingsTab::About => EpIcon::InfoFilled,
        }
    }
}

pub const WIDTH: f32 = 680.;
pub const HEIGHT: f32 = 560.;

/// 同步按钮在本窗口内的防重入（旧 `syncAction`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncAction {
    Sync,
    Pull,
    Push,
}

/// 设置窗口视图
pub struct SettingsWindow {
    pub tab: SettingsTab,
    state: Entity<AppState>,
    scroll: ScrollHandle,

    // ---- 常规 ----
    auto_start: bool,
    auto_start_loading: bool,
    /// 背景透明度拖动中的预览值（%）
    alpha_preview: Option<f32>,
    notification_switching: bool,
    bg_picker: Entity<EpColorPicker>,

    // ---- 外观 ----
    font_select: Entity<EpSelect>,
    /// 字体大小拖动中的预览值
    font_size_preview: Option<i32>,

    // ---- 数据与同步 ----
    exporting: bool,
    importing: bool,
    url_input: Entity<InputState>,
    /// 地址输入框悬停（`clearable` 只在悬停 / 聚焦且非空时显示清空图标）
    url_hovered: bool,
    user_input: Entity<InputState>,
    password_input: Entity<InputState>,
    password_visible: bool,
    has_password: bool,
    auto_sync: bool,
    sync_interval: i32,
    last_sync_at: Option<String>,
    device_id: String,
    interval_select: Entity<EpSelect>,
    testing: bool,
    clearing_password: bool,
    sync_action: Option<SyncAction>,
    advanced_open: bool,
    /// 已处理过的同步报告（AppState 里出现新报告时刷新「上次同步」）
    seen_report: Option<SyncReport>,

    // ---- 屏幕配置 ----
    current_screen_id: String,

    // ---- 关于 ----
    checking_update: bool,

    _subscriptions: Vec<Subscription>,
}

/// 同步间隔选项（分钟）
const SYNC_INTERVALS: [i32; 5] = [5, 10, 15, 30, 60];

impl SettingsWindow {
    pub fn new(tab: SettingsTab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let url_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://dav.example.com/dav"));
        let user_input = cx.new(|cx| InputState::new(window, cx).placeholder("用户名"));
        let password_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("密码").masked(true));
        let bg_picker = cx.new(|cx| EpColorPicker::new("bg-color-picker", window, cx));
        let font_select = cx.new(|cx| {
            EpSelect::new("font-select", window, cx)
                .filterable()
                .clearable()
                .placeholder("跟随系统")
        });
        let interval_select = cx.new(|cx| {
            EpSelect::new("interval-select", window, cx)
                .small()
                .with_options(
                    SYNC_INTERVALS
                        .iter()
                        .map(|m| EpSelectOption::new(m.to_string(), format!("{m} 分钟")))
                        .collect(),
                )
        });

        let subscriptions = vec![
            cx.observe(&state, |this, _, cx| {
                this.sync_from_state(cx);
                cx.notify();
            }),
            cx.observe_global::<WindowManager>(|_, cx| cx.notify()),
            cx.subscribe_in(&url_input, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    // http:// 明文提示随输入实时出现 / 消失
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &bg_picker,
                window,
                |this, _, event: &EpColorPickerEvent, window, cx| match event {
                    EpColorPickerEvent::Change(color) => {
                        // 点 Clear 按默认黑色处理
                        let color = color
                            .as_ref()
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| mini_todo_core::db::DEFAULT_WINDOW_BG_COLOR.into());
                        this.set_bg_color(color, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &font_select,
                window,
                |this, _, event: &EpSelectEvent, window, cx| match event {
                    EpSelectEvent::Change(value) => this.set_font_family(value.clone(), window, cx),
                },
            ),
            cx.subscribe_in(
                &interval_select,
                window,
                |this, _, event: &EpSelectEvent, window, cx| match event {
                    EpSelectEvent::Change(Some(value)) => {
                        if let Ok(minutes) = value.parse::<i32>() {
                            this.sync_interval = minutes;
                            this.save_sync_settings(window, cx);
                        }
                    }
                    EpSelectEvent::Change(None) => {}
                },
            ),
        ];

        let mut this = Self {
            tab,
            state,
            scroll: ScrollHandle::new(),
            auto_start: false,
            auto_start_loading: false,
            alpha_preview: None,
            notification_switching: false,
            bg_picker,
            font_select,
            font_size_preview: None,
            exporting: false,
            importing: false,
            url_input,
            url_hovered: false,
            user_input,
            password_input,
            password_visible: false,
            has_password: false,
            auto_sync: false,
            sync_interval: 15,
            last_sync_at: None,
            device_id: String::new(),
            interval_select,
            testing: false,
            clearing_password: false,
            sync_action: None,
            advanced_open: false,
            seen_report: None,
            current_screen_id: crate::platform::screen_config_id(cx),
            checking_update: false,
            _subscriptions: subscriptions,
        };
        this.seen_report = this.state.read(cx).last_sync_report.clone();
        this.sync_from_state(cx);
        this.load(window, cx);
        this
    }

    /// 打开时读取（旧 `onMounted`）：开机自启、屏幕配置、应用设置、同步设置、系统字体
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_autostart(cx);
        self.state.update(cx, |state, cx| {
            state.reload_screen_configs(cx).detach();
            state.reload_app_settings(cx).detach();
        });
        self.load_sync_settings(window, cx);
        self.load_system_fonts(cx);
    }

    /// AppState 变化后同步子控件（底色选择器、字体下拉）与「上次同步」
    fn sync_from_state(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let color = state.window_bg.color.clone();
        let dark = state.is_dark_theme;
        let family = state.todo_font_family.clone();
        let report = state.last_sync_report.clone();
        self.bg_picker.update(cx, |picker, cx| {
            picker.set_value(color, cx);
            picker.set_disabled(!dark, cx);
        });
        self.font_select.update(cx, |select, cx| {
            let value = (!family.trim().is_empty()).then(|| SharedString::from(family));
            select.set_selected(value, cx);
        });
        if report.is_some() && report != self.seen_report {
            if let Some(report) = &report {
                self.last_sync_at = Some(report.last_sync_at.clone());
            }
            self.seen_report = report;
        }
    }

    fn select_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        }
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = settings_tokens();
        div()
            .flex_none()
            .w(px(160.))
            .h_full()
            .p(px(8.))
            .bg(t.surface)
            .border_r_1()
            .border_color(t.border)
            .children(SettingsTab::ALL.iter().map(|&tab| {
                let active = self.tab == tab;
                let color = if active { t.primary } else { t.secondary };
                let group = SharedString::from(format!("settings-menu-{}", tab.key()));
                div()
                    .id(SharedString::from(format!("menu-{}", tab.key())))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(theme::radius::MD)
                    .text_size(px(13.))
                    .font_weight(theme::WEIGHT_MEDIUM)
                    .text_color(color)
                    .cursor_pointer()
                    .when(active, |this| this.bg(t.active_bg))
                    .when(!active, |this| {
                        this.hover(move |s| s.bg(t.border_light).text_color(t.label))
                    })
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.select_tab(tab, cx)),
                    )
                    .child(icon(tab.icon(), 16., color).when(!active, |svg| {
                        svg.group_hover(group.clone(), move |s| s.text_color(t.label))
                    }))
                    .child(tab.label())
            }))
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = settings_tokens();
        let panel = match self.tab {
            SettingsTab::General => self.render_general(window, cx).into_any_element(),
            SettingsTab::Appearance => self.render_appearance(window, cx).into_any_element(),
            SettingsTab::Data => self.render_data(window, cx).into_any_element(),
            SettingsTab::Screen => self.render_screen(window, cx).into_any_element(),
            SettingsTab::About => self.render_about(window, cx).into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .bg(t.window_bg)
            .text_color(t.label)
            .child(
                WindowTitleBar::new("settings-title", "设置")
                    .title_size(17.)
                    .padding(12., 20.)
                    .min_h(px(57.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_menu(cx))
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(div().px(px(24.)).py(px(20.)).child(panel)),
                    ),
            )
    }
}

// ----------------------------------------------------------------------
// 公共排版（`.panel-title` / `.settings-row` / `.card-hint` / `.data-btn` …）
// ----------------------------------------------------------------------

/// `.panel-title`：15px / 600 / `#1E293B`，下边距 16
fn panel_title(text: impl Into<SharedString>) -> Div {
    let t = settings_tokens();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mb(px(16.))
        .text_size(px(15.))
        .font_weight(theme::WEIGHT_SEMIBOLD)
        .text_color(t.title)
        .child(text.into())
}

/// `.settings-row`：左（18px 图标 + 标签 / 说明）右（控件），`padding 12px 0`、下边线 `#F1F5F9`；
/// `last` = 面板最后一行（无下边线、无下内边距）
fn settings_row(
    icon_source: EpIcon,
    label: &'static str,
    desc: Option<&'static str>,
    control: impl IntoElement,
    last: bool,
) -> Div {
    let t = settings_tokens();
    div()
        .flex()
        .items_center()
        .justify_between()
        .pt(px(12.))
        .when(!last, |this| {
            this.pb(px(12.)).border_b_1().border_color(t.border_light)
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(12.))
                .child(icon(icon_source, 18., t.secondary))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(theme::WEIGHT_MEDIUM)
                                .text_color(t.label)
                                .child(label),
                        )
                        .when_some(desc, |this, desc| {
                            this.child(
                                div()
                                    .mt(px(2.))
                                    .text_size(px(12.))
                                    .text_color(t.tertiary)
                                    .child(desc),
                            )
                        }),
                ),
        )
        .child(div().flex_none().child(control))
}

/// `.card-hint`（`warning` = `.warning-hint`：文字 `#B45309`、图标 `#F59E0B`、上边距 4）
fn card_hint(icon_source: EpIcon, text: impl Into<SharedString>, warning: bool) -> Div {
    let t = settings_tokens();
    let (text_color, icon_color) = if warning {
        (t.warning_text, t.warning_icon)
    } else {
        (t.secondary, t.tertiary)
    };
    div()
        .flex()
        .items_start()
        .gap(px(6.))
        .when(warning, |this| this.mt(px(4.)))
        .text_size(px(12.))
        .text_color(text_color)
        .child(
            div()
                .mt(px(1.))
                .flex_none()
                .child(icon(icon_source, 14., icon_color)),
        )
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// `.section-divider`
fn section_divider() -> Div {
    div().h(px(1.)).my(px(20.)).bg(settings_tokens().border)
}

/// `.form-label`：13px / 500 / `#64748B`，下边距 8
fn form_label() -> Div {
    let t = settings_tokens();
    div()
        .flex()
        .items_center()
        .justify_between()
        .mb(px(8.))
        .text_size(px(13.))
        .font_weight(theme::WEIGHT_MEDIUM)
        .text_color(t.secondary)
}

/// `.data-btn` 变体
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataBtnKind {
    Normal,
    Primary,
    Danger,
}

/// `.data-btn`：`flex:1`、居中、gap 8、`padding 10px 16px`、1px 边、圆角 8、13px / 500、图标 16；禁用 `opacity .6`
fn data_btn(
    id: &'static str,
    icon_source: EpIcon,
    label: impl Into<SharedString>,
    kind: DataBtnKind,
    disabled: bool,
    busy: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let t = settings_tokens();
    let (bg, border, text, hover_bg, hover_border) = match kind {
        DataBtnKind::Normal => (t.surface, t.border, t.label, t.window_bg, t.hover_border),
        DataBtnKind::Primary => (
            t.primary,
            t.primary,
            t.surface,
            t.primary_dark,
            t.primary_dark,
        ),
        DataBtnKind::Danger => (
            t.surface,
            t.danger_border,
            t.danger_text,
            t.danger_hover_bg,
            t.danger_hover_border,
        ),
    };
    let glyph: AnyElement = if busy {
        spinning(
            (gpui_kit::ElementId::from(id), "spin"),
            icon(EpIcon::Loading, 16., text),
        )
        .into_any_element()
    } else {
        icon(icon_source, 16., text).into_any_element()
    };
    div()
        .id(id)
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(10.))
        .border_1()
        .border_color(border)
        .rounded(theme::radius::MD)
        .bg(bg)
        .text_size(px(13.))
        .font_weight(theme::WEIGHT_MEDIUM)
        .text_color(text)
        .when(disabled, |this| {
            this.opacity(0.6)
                .cursor(gpui_kit::CursorStyle::OperationNotAllowed)
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(move |s| s.bg(hover_bg).border_color(hover_border))
                .on_click(on_click)
        })
        .child(glyph)
        .child(label.into())
}

/// 打开设置窗口（打开方：主窗口；截图调试 `None`）
pub fn open(cx: &mut App, parent: Option<ChildOf>, tab: SettingsTab) -> Option<AnyWindowHandle> {
    let mut spec = WindowSpec::new(
        "设置",
        gpui_kit::Bounds::new(Default::default(), logical_size(WIDTH, HEIGHT)),
    );
    spec.resizable = false;
    WindowManager::open_child(
        cx,
        parent.as_ref(),
        WindowKind::Settings,
        None,
        spec,
        |window, cx| cx.new(|cx| SettingsWindow::new(tab, window, cx)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_round_trip() {
        for tab in SettingsTab::ALL {
            assert_eq!(SettingsTab::parse(tab.key()), Some(tab));
        }
        assert_eq!(SettingsTab::Data.label(), "数据与同步");
        assert_eq!(SettingsTab::parse("bogus"), None);
    }
}
