//! 设置窗口（旧 `SettingsView.vue`）。**W3 实现**；现在是占位视图。
//!
//! 规格：`research/ui-settings-fence.md` §2。窗口 680×560、**不可缩放**、不透明，打开方是主窗口（owner）；
//! 入口：标题栏齿轮、托盘「打开设置」。关闭后主窗口刷新待办、重读设置、按新设置重建自动同步。

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, IntoElement, Render, SharedString, Window,
};

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size, placeholder};

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
}

pub const WIDTH: f32 = 680.;
pub const HEIGHT: f32 = 560.;

/// 设置窗口视图（W3 替换内容）
pub struct SettingsWindow {
    pub tab: SettingsTab,
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        placeholder(
            "settings-title",
            SharedString::from("设置"),
            SharedString::from(format!("设置窗口（W3 实现）当前：{}", self.tab.label())),
        )
    }
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
        |_, cx| cx.new(|_| SettingsWindow { tab }),
    )
}
