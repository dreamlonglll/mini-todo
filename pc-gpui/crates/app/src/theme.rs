//! 颜色 / 字体 / 圆角 / 间距 / 阴影 / 动画时长 token。**界面上的颜色与尺寸只从这里取**，数值照抄
//! `research/ui-main-window.md` §2 与 `ui-editor-windows.md` §1.2（单位：逻辑像素）。
//!
//! 两套调色板：
//! - [`Palette`]：应用自己的 CSS 变量（`--primary` `#3B82F6` 系）。浅色 [`light`]；深色 [`Palette::dark`]
//!   只用于主窗口（深色主题只在主窗口生效，其它窗口永远是浅色）。
//! - [`EpPalette`]：Element Plus 默认调色板（`#409EFF` 系），EP 风格控件（`ui::button` 等）用它，
//!   [`init`] 也用它把 gpui-component 的主题调成 Element Plus 外观。
//!
//! 深色主题的"实际效果"以规格文档里的**最终值**为准（CSS 的 `!important` 层叠已解析），不要按 SCSS 重推。

use std::sync::LazyLock;
use std::time::Duration;

use gpui_kit::component::Theme;
use gpui_kit::{
    Anchor, App, BoxShadow, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, Hsla, Pixels,
    SharedString, px, rgb, rgba,
};

/// `0xRRGGBB` → 颜色
pub fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

/// `0xRRGGBBAA` → 颜色
pub fn hexa(value: u32) -> Hsla {
    rgba(value).into()
}

/// CSS `rgba(r, g, b, a)`（r/g/b 0~255，a 0~1）
pub fn rgba_css(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    gpui_kit::Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: a.clamp(0.0, 1.0),
    }
    .into()
}

/// 前端 `hexToRgbChannels` 的等价物：`#RGB` / `#RRGGBB`（大小写均可，`#` 可省）→ `(r, g, b)`；
/// 非法输入回落 `(0, 0, 0)`。
pub fn hex_to_rgb_channels(color: &str) -> (u8, u8, u8) {
    parse_hex_color(color).unwrap_or((0, 0, 0))
}

/// HEX 颜色比较（旧 `utils/color.ts` 的 `isSameColor`：去空白、忽略大小写）
pub fn is_same_color(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// 解析 `#RGB` / `#RRGGBB`；非法返回 `None`
pub fn parse_hex_color(color: &str) -> Option<(u8, u8, u8)> {
    let s = color.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&s[i..i + 1], 16).ok();
    match s.len() {
        3 => Some((digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
        6 => Some((
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
        )),
        _ => None,
    }
}

/// 待办颜色字段（`todo.color`，如 `#EF4444`）→ 颜色；非法时用 `fallback`
pub fn todo_color(color: &str, fallback: Hsla) -> Hsla {
    parse_hex_color(color).map_or(fallback, |(r, g, b)| rgba_css(r, g, b, 1.0))
}

/// 深色主题下主窗口的底色（settings `window_bg_color` / `window_bg_alpha`，默认 `#000000` / 0.45）
#[derive(Debug, Clone, PartialEq)]
pub struct WindowBackground {
    pub color: String,
    pub alpha: f64,
}

impl Default for WindowBackground {
    fn default() -> Self {
        Self {
            color: mini_todo_core::db::DEFAULT_WINDOW_BG_COLOR.to_string(),
            alpha: mini_todo_core::db::DEFAULT_WINDOW_BG_ALPHA,
        }
    }
}

impl WindowBackground {
    /// `rgba(var(--app-bg-rgb), var(--app-bg-alpha))`
    pub fn to_hsla(&self) -> Hsla {
        let (r, g, b) = hex_to_rgb_channels(&self.color);
        rgba_css(r, g, b, self.alpha as f32)
    }
}

/// 设置窗口「界面底色」的预设色（`types/app.ts PRESET_BG_COLORS`）
pub const PRESET_BG_COLORS: &[(&str, &str)] = &[
    ("黑色", "#000000"),
    ("深灰", "#1E293B"),
    ("深蓝", "#0F2942"),
    ("深紫", "#2E1065"),
    ("深绿", "#052E24"),
    ("深棕", "#2B1A0F"),
];

/// 应用 CSS 变量（ui-main-window.md §2.1）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub is_dark: bool,
    /// `--primary` `#3B82F6`
    pub primary: Hsla,
    /// `--primary-light` `#60A5FA`
    pub primary_light: Hsla,
    /// `--primary-dark` `#2563EB`
    pub primary_dark: Hsla,
    /// `--bg-primary`：浅色 transparent；深色 = 用户底色
    pub bg_primary: Hsla,
    /// `--bg-secondary`：浅色 `#F8FAFC`；深色 = 用户底色
    pub bg_secondary: Hsla,
    /// `--bg-tertiary`：浅色 `#F1F5F9`；深色 `rgba(255,255,255,0.1)`
    pub bg_tertiary: Hsla,
    /// `--text-primary`：浅色 `#1E293B`；深色 `#FFFFFF`
    pub text_primary: Hsla,
    /// `--text-secondary`：浅色 `#64748B`；深色 `rgba(255,255,255,0.85)`
    pub text_secondary: Hsla,
    /// `--text-tertiary`：浅色 `#94A3B8`；深色 `rgba(255,255,255,0.65)`
    pub text_tertiary: Hsla,
    /// `--border`：浅色 `#E2E8F0`；深色 `rgba(255,255,255,0.2)`
    pub border: Hsla,
    /// `--border-light`：浅色 `#F1F5F9`；深色 `rgba(255,255,255,0.1)`
    pub border_light: Hsla,
    /// `--success` `#10B981`
    pub success: Hsla,
    /// `--warning` `#F59E0B`
    pub warning: Hsla,
    /// `--error` `#EF4444`
    pub error: Hsla,
    /// 浅色主题主窗口 `.app-container` 底色 `#FFFFFF`；深色下为用户底色（只画一层）
    pub window_bg: Hsla,
    /// 主窗口外描边：浅色 `rgba(0,0,0,0.5)`；深色 `rgba(255,255,255,0.5)`
    pub window_border: Hsla,
}

static LIGHT: LazyLock<Palette> = LazyLock::new(|| Palette {
    is_dark: false,
    primary: hex(0x3B82F6),
    primary_light: hex(0x60A5FA),
    primary_dark: hex(0x2563EB),
    bg_primary: hexa(0x00000000),
    bg_secondary: hex(0xF8FAFC),
    bg_tertiary: hex(0xF1F5F9),
    text_primary: hex(0x1E293B),
    text_secondary: hex(0x64748B),
    text_tertiary: hex(0x94A3B8),
    border: hex(0xE2E8F0),
    border_light: hex(0xF1F5F9),
    success: hex(0x10B981),
    warning: hex(0xF59E0B),
    error: hex(0xEF4444),
    window_bg: hex(0xFFFFFF),
    window_border: rgba_css(0, 0, 0, 0.5),
});

/// 浅色调色板（编辑 / 子任务 / 设置 / 已完成 / 通知窗口永远用它；主窗口浅色主题也用它）
pub fn light() -> &'static Palette {
    &LIGHT
}

impl Palette {
    /// 主窗口深色主题（Windows / Linux 的最终值；`bg` 为用户底色与透明度）
    pub fn dark(bg: &WindowBackground) -> Palette {
        let user_bg = bg.to_hsla();
        Palette {
            is_dark: true,
            bg_primary: user_bg,
            bg_secondary: user_bg,
            bg_tertiary: rgba_css(255, 255, 255, 0.1),
            text_primary: hex(0xFFFFFF),
            text_secondary: rgba_css(255, 255, 255, 0.85),
            text_tertiary: rgba_css(255, 255, 255, 0.65),
            border: rgba_css(255, 255, 255, 0.2),
            border_light: rgba_css(255, 255, 255, 0.1),
            window_bg: user_bg,
            window_border: rgba_css(255, 255, 255, 0.5),
            ..*light()
        }
    }

    /// 主窗口用的调色板
    pub fn for_main(is_dark: bool, bg: &WindowBackground) -> Palette {
        if is_dark { Palette::dark(bg) } else { *light() }
    }

    /// `--shadow-sm`（深色主题下 none）
    pub fn shadow_sm(&self) -> Vec<BoxShadow> {
        if self.is_dark {
            return vec![];
        }
        vec![shadow(0., 1., 2., 0., rgba_css(0, 0, 0, 0.05))]
    }

    /// `--shadow-base`
    pub fn shadow_base(&self) -> Vec<BoxShadow> {
        if self.is_dark {
            return vec![];
        }
        vec![
            shadow(0., 1., 3., 0., rgba_css(0, 0, 0, 0.1)),
            shadow(0., 1., 2., 0., rgba_css(0, 0, 0, 0.06)),
        ]
    }

    /// `--shadow-md`
    pub fn shadow_md(&self) -> Vec<BoxShadow> {
        if self.is_dark {
            return vec![];
        }
        vec![
            shadow(0., 4., 6., -1., rgba_css(0, 0, 0, 0.1)),
            shadow(0., 2., 4., -1., rgba_css(0, 0, 0, 0.06)),
        ]
    }

    /// `--shadow-lg`
    pub fn shadow_lg(&self) -> Vec<BoxShadow> {
        if self.is_dark {
            return vec![];
        }
        vec![
            shadow(0., 10., 15., -3., rgba_css(0, 0, 0, 0.1)),
            shadow(0., 4., 6., -2., rgba_css(0, 0, 0, 0.05)),
        ]
    }
}

/// CSS `box-shadow: x y blur spread color`
pub fn shadow(x: f32, y: f32, blur: f32, spread: f32, color: Hsla) -> BoxShadow {
    BoxShadow::new(px(x), px(y), color)
        .blur_radius(px(blur))
        .spread_radius(px(spread))
}

/// Element Plus 2.13 默认调色板（项目没有改 EP 主题色，所以 EP 控件是 `#409EFF`，与应用自绘的 `#3B82F6` 并存）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EpPalette {
    /// `#409EFF`
    pub primary: Hsla,
    /// light-3 `#79BBFF`（primary 按钮 hover）
    pub primary_light_3: Hsla,
    /// light-5 `#A0CFFF`（primary 按钮 disabled；focus-visible 外框 `rgb(160,207,255)`）
    pub primary_light_5: Hsla,
    /// light-7 `#C6E2FF`（默认按钮 hover 边框）
    pub primary_light_7: Hsla,
    /// light-8 `#D9ECFF`
    pub primary_light_8: Hsla,
    /// light-9 `#ECF5FF`（默认按钮 hover 底 / 下拉菜单 hover 底）
    pub primary_light_9: Hsla,
    /// dark-2 `#337ECC`（primary 按钮 active）
    pub primary_dark_2: Hsla,
    /// `#67C23A`
    pub success: Hsla,
    pub success_light_5: Hsla,
    pub success_light_8: Hsla,
    pub success_light_9: Hsla,
    pub success_dark_2: Hsla,
    /// `#E6A23C`
    pub warning: Hsla,
    pub warning_light_5: Hsla,
    pub warning_light_8: Hsla,
    pub warning_light_9: Hsla,
    pub warning_dark_2: Hsla,
    /// `#F56C6C`
    pub danger: Hsla,
    pub danger_light_8: Hsla,
    pub danger_light_9: Hsla,
    /// `#909399`
    pub info: Hsla,
    pub info_light_8: Hsla,
    pub info_light_9: Hsla,
    /// 文字：primary `#303133` / regular `#606266` / secondary `#909399` / placeholder `#A8ABB2` / disabled `#C0C4CC`
    pub text_primary: Hsla,
    pub text_regular: Hsla,
    pub text_secondary: Hsla,
    pub text_placeholder: Hsla,
    pub text_disabled: Hsla,
    /// 边框：`#DCDFE6` / light `#E4E7ED` / lighter `#EBEEF5`；输入框 hover `#C0C4CC`
    pub border: Hsla,
    pub border_light: Hsla,
    pub border_lighter: Hsla,
    pub border_hover: Hsla,
    /// 填充：`#F0F2F5` / light `#F5F7FA` / blank `#FFFFFF`
    pub fill: Hsla,
    pub fill_light: Hsla,
    pub fill_blank: Hsla,
    /// 禁用：底 `#F5F7FA`、边 `#E4E7ED`、字 `#A8ABB2`
    pub disabled_bg: Hsla,
    pub disabled_border: Hsla,
    pub disabled_text: Hsla,
    /// 对话框 / 确认框遮罩 `rgba(0,0,0,.5)`
    pub overlay: Hsla,
    /// tooltip（dark）底 `#303133`
    pub tooltip_bg: Hsla,
    pub white: Hsla,
}

static EP: LazyLock<EpPalette> = LazyLock::new(|| EpPalette {
    primary: hex(0x409EFF),
    primary_light_3: hex(0x79BBFF),
    primary_light_5: hex(0xA0CFFF),
    primary_light_7: hex(0xC6E2FF),
    primary_light_8: hex(0xD9ECFF),
    primary_light_9: hex(0xECF5FF),
    primary_dark_2: hex(0x337ECC),
    success: hex(0x67C23A),
    success_light_5: hex(0xB3E19D),
    success_light_8: hex(0xE1F3D8),
    success_light_9: hex(0xF0F9EB),
    success_dark_2: hex(0x529B2E),
    warning: hex(0xE6A23C),
    warning_light_5: hex(0xF3D19E),
    warning_light_8: hex(0xFAECD8),
    warning_light_9: hex(0xFDF6EC),
    warning_dark_2: hex(0xB88230),
    danger: hex(0xF56C6C),
    danger_light_8: hex(0xFDE2E2),
    danger_light_9: hex(0xFEF0F0),
    info: hex(0x909399),
    info_light_8: hex(0xE9E9EB),
    info_light_9: hex(0xF4F4F5),
    text_primary: hex(0x303133),
    text_regular: hex(0x606266),
    text_secondary: hex(0x909399),
    text_placeholder: hex(0xA8ABB2),
    text_disabled: hex(0xC0C4CC),
    border: hex(0xDCDFE6),
    border_light: hex(0xE4E7ED),
    border_lighter: hex(0xEBEEF5),
    border_hover: hex(0xC0C4CC),
    fill: hex(0xF0F2F5),
    fill_light: hex(0xF5F7FA),
    fill_blank: hex(0xFFFFFF),
    disabled_bg: hex(0xF5F7FA),
    disabled_border: hex(0xE4E7ED),
    disabled_text: hex(0xA8ABB2),
    overlay: rgba_css(0, 0, 0, 0.5),
    tooltip_bg: hex(0x303133),
    white: hex(0xFFFFFF),
});

/// Element Plus 调色板
pub fn ep() -> &'static EpPalette {
    &EP
}

impl EpPalette {
    /// `--el-box-shadow`：`0 12px 32px 4px rgba(0,0,0,.04), 0 8px 20px rgba(0,0,0,.08)`（对话框 / 确认框）
    pub fn box_shadow(&self) -> Vec<BoxShadow> {
        vec![
            shadow(0., 12., 32., 4., rgba_css(0, 0, 0, 0.04)),
            shadow(0., 8., 20., 0., rgba_css(0, 0, 0, 0.08)),
        ]
    }

    /// `--el-box-shadow-light`：`0 0 12px rgba(0,0,0,.12)`（下拉 / 弹出层）
    pub fn box_shadow_light(&self) -> Vec<BoxShadow> {
        vec![shadow(0., 0., 12., 0., rgba_css(0, 0, 0, 0.12))]
    }
}

/// 圆角（`--radius-*`）
pub mod radius {
    use gpui_kit::{Pixels, px};
    /// `--radius-sm` 4px（也是 Element Plus 弹层 / tooltip / 消息的圆角）
    pub const SM: Pixels = px(4.);
    /// `--radius-base` 6px（main.scss 把 el-button / el-input 统一覆盖为 6px）
    pub const BASE: Pixels = px(6.);
    /// `--radius-md` 8px（主窗口外框）
    pub const MD: Pixels = px(8.);
    /// `--radius-lg` 12px（el-dialog 覆盖值）
    pub const LG: Pixels = px(12.);
    /// `--radius-xl` 16px
    pub const XL: Pixels = px(16.);
}

/// 间距（`--space-*`）
pub mod space {
    use gpui_kit::{Pixels, px};
    pub const S1: Pixels = px(4.);
    pub const S2: Pixels = px(8.);
    pub const S3: Pixels = px(12.);
    pub const S4: Pixels = px(16.);
    pub const S5: Pixels = px(20.);
    pub const S6: Pixels = px(24.);
}

/// 过渡时长（`--transition-*`）
pub mod duration {
    use std::time::Duration;
    pub const FAST: Duration = Duration::from_millis(150);
    pub const BASE: Duration = Duration::from_millis(200);
    pub const SLOW: Duration = Duration::from_millis(300);
}

/// 全局字号 14px
pub const FONT_SIZE: Pixels = px(14.);
/// 全局行高 1.5
pub const LINE_HEIGHT: f32 = 1.5;
/// Element Plus 组件尺寸：default 32 / small 24
pub const EP_HEIGHT_DEFAULT: Pixels = px(32.);
pub const EP_HEIGHT_SMALL: Pixels = px(24.);
/// tooltip 显示延迟：原生 title 约 500ms；el-tooltip 按调用（300 / 500ms），隐藏 200ms
pub const TOOLTIP_HIDE_AFTER: Duration = Duration::from_millis(200);

/// 界面字体族：Windows `Segoe UI`（拉丁）+ `Microsoft YaHei`（中文回退），与 CSS
/// `'Segoe UI', 'Microsoft YaHei', sans-serif` 一致。Linux（只用于 CI 与截图）用度量最接近的
/// `Liberation Sans` + `Noto Sans CJK SC`（与参考截图 harness 的替身字体相同）。
pub fn font_family() -> SharedString {
    if cfg!(windows) {
        "Segoe UI".into()
    } else {
        "Liberation Sans".into()
    }
}

/// 中文回退字体
pub fn font_fallbacks() -> FontFallbacks {
    if cfg!(windows) {
        FontFallbacks::from_fonts(vec!["Microsoft YaHei".into(), "Microsoft YaHei UI".into()])
    } else {
        FontFallbacks::from_fonts(vec!["Noto Sans CJK SC".into()])
    }
}

/// 界面字体（字重 Regular）；窗口根元素 `.font(theme::ui_font())`
pub fn ui_font() -> Font {
    Font {
        family: font_family(),
        features: FontFeatures::default(),
        fallbacks: Some(font_fallbacks()),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

/// 用户自选的待办字体（设置里 `todo_font_family`，空 = 默认字体栈）：
/// `"<family>", -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif`
pub fn todo_font(family: &str) -> Font {
    let family = family.trim();
    if family.is_empty() {
        return ui_font();
    }
    let mut fallbacks = vec![font_family().to_string()];
    fallbacks.extend(font_fallbacks().fallback_list().iter().cloned());
    Font {
        family: family.to_string().into(),
        features: FontFeatures::default(),
        fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

/// CSS `font-weight: 600`：Segoe UI 有 Semibold，微软雅黑回落 Bold
pub const WEIGHT_SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;
/// CSS `font-weight: 500`：Segoe UI / 微软雅黑没有 500，实际渲染为 Regular
pub const WEIGHT_MEDIUM: FontWeight = FontWeight::NORMAL;

/// 把 gpui-component 的主题调成 Element Plus 外观（输入框、下拉、复选框、开关、日期选择、弹层等组件用）。
/// 必须在 `gpui_kit::init` 之后、开窗之前调用。
pub fn init(cx: &mut App) {
    let ep = ep();
    Theme::update(cx, |t| {
        t.font_family = font_family();
        t.font_size = FONT_SIZE;
        // main.scss：el-button / el-input 圆角覆盖为 6px；EP 弹层 / 下拉 / 消息 4px
        t.radius = radius::BASE;
        t.radius_lg = radius::SM;
        // EP 没有焦点外环：聚焦只改边框色
        t.focus_ring = false;
        t.shadow = true;
        t.notification.placement = Anchor::TopCenter;

        t.background = ep.white;
        t.foreground = ep.text_regular;
        t.muted = ep.fill_light;
        t.muted_foreground = ep.text_placeholder;
        t.border = ep.border;
        t.input = ep.border;
        t.ring = ep.primary;
        t.caret = ep.text_regular;
        t.selection = ep.primary.opacity(0.25);
        t.popover = ep.white;
        t.popover_foreground = ep.text_regular;
        t.accent = ep.fill_light;
        t.accent_foreground = ep.text_regular;
        t.link = ep.primary;
        t.link_hover = ep.primary_light_3;
        t.link_active = ep.primary_dark_2;

        t.primary = ep.primary;
        t.primary_hover = ep.primary_light_3;
        t.primary_active = ep.primary_dark_2;
        t.primary_foreground = ep.white;
        t.button_primary = ep.primary;
        t.button_primary_hover = ep.primary_light_3;
        t.button_primary_active = ep.primary_dark_2;
        t.button_primary_foreground = ep.white;

        t.success = ep.success;
        t.success_foreground = ep.white;
        t.warning = ep.warning;
        t.warning_foreground = ep.white;
        t.danger = ep.danger;
        t.danger_foreground = ep.white;
        t.info = ep.info;
        t.info_foreground = ep.white;

        t.switch = ep.border;
        t.switch_thumb = ep.white;
        t.slider_bar = ep.primary;
        t.slider_thumb = ep.white;
        t.overlay = ep.overlay;
        t.list_active = ep.primary_light_9;
        t.list_active_border = ep.primary;
        t.list_hover = ep.fill_light;
    });
}

// ---- 主窗口（`research/ui-main-window.md`，深色值为层叠后的最终值） ----

/// 主窗口专用颜色（由 [`Palette`] 推出浅 / 深两套；规格 §5~§10）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MainColors {
    /// 待办行 hover 底：浅 `rgba(0,0,0,0.03)`；深色无变化（透明）
    pub row_hover: Hsla,
    /// 拖拽占位底：浅 `rgba(59,130,246,0.1)`；深色透明
    pub row_dragging: Hsla,
    /// 操作条底：浅 `rgba(255,255,255,0.95)` / 深 `rgba(30,30,30,0.9)`
    pub actions_bg: Hsla,
    /// 操作条阴影色：浅 `rgba(0,0,0,0.1)` / 深 `rgba(0,0,0,0.3)`（`0 1px 4px`）
    pub actions_shadow: Hsla,
    /// 子任务计数 hover 底（深色下也是这个值）`rgba(0,0,0,0.05)`
    pub subtask_count_hover: Hsla,
    /// 空状态底：深色 `rgba(0,0,0,0.1)`，浅色透明
    pub empty_bg: Hsla,
    /// 四象限格底（深色统一 `rgba(0,0,0,0.15)`；浅色按象限另算）、边框、hover 边框、头部底、头部下边框、计数胶囊
    pub quadrant_dark_bg: Hsla,
    pub quadrant_border: Hsla,
    pub quadrant_border_hover_dark: Hsla,
    pub quadrant_header_bg: Hsla,
    pub quadrant_header_border: Hsla,
    pub quadrant_count_bg: Hsla,
    pub quadrant_count_fg: Hsla,
    pub quadrant_title: Hsla,
    /// 日历网格线：浅 `#E2E8F0` / 深 `rgba(255,255,255,0.6)`
    pub calendar_line: Hsla,
    /// 日历导航按钮：颜色、hover 色、hover 底
    pub nav_fg: Hsla,
    pub nav_hover_fg: Hsla,
    pub nav_hover_bg: Hsla,
    /// 「今天」按钮：字 / 底 / 边，hover 字 / 底 / 边
    pub today_fg: Hsla,
    pub today_bg: Hsla,
    pub today_border: Hsla,
    pub today_hover_fg: Hsla,
    pub today_hover_bg: Hsla,
    pub today_hover_border: Hsla,
}

impl MainColors {
    pub fn new(p: &Palette) -> MainColors {
        let clear = hexa(0x00000000);
        if p.is_dark {
            MainColors {
                row_hover: clear,
                row_dragging: clear,
                actions_bg: rgba_css(30, 30, 30, 0.9),
                actions_shadow: rgba_css(0, 0, 0, 0.3),
                subtask_count_hover: rgba_css(0, 0, 0, 0.05),
                empty_bg: rgba_css(0, 0, 0, 0.1),
                quadrant_dark_bg: rgba_css(0, 0, 0, 0.15),
                quadrant_border: rgba_css(255, 255, 255, 0.1),
                quadrant_border_hover_dark: rgba_css(255, 255, 255, 0.2),
                quadrant_header_bg: rgba_css(0, 0, 0, 0.1),
                quadrant_header_border: rgba_css(128, 128, 128, 0.1),
                quadrant_count_bg: rgba_css(255, 255, 255, 0.1),
                quadrant_count_fg: p.text_secondary,
                quadrant_title: p.text_primary,
                calendar_line: rgba_css(255, 255, 255, 0.6),
                nav_fg: hex(0xFFFFFF),
                nav_hover_fg: p.primary_light,
                nav_hover_bg: rgba_css(255, 255, 255, 0.1),
                today_fg: hex(0xFFFFFF),
                today_bg: clear,
                today_border: rgba_css(255, 255, 255, 0.4),
                today_hover_fg: hex(0xFFFFFF),
                today_hover_bg: rgba_css(255, 255, 255, 0.15),
                today_hover_border: rgba_css(255, 255, 255, 0.5),
            }
        } else {
            MainColors {
                row_hover: rgba_css(0, 0, 0, 0.03),
                row_dragging: rgba_css(59, 130, 246, 0.1),
                actions_bg: rgba_css(255, 255, 255, 0.95),
                actions_shadow: rgba_css(0, 0, 0, 0.1),
                subtask_count_hover: rgba_css(0, 0, 0, 0.05),
                empty_bg: clear,
                quadrant_dark_bg: rgba_css(0, 0, 0, 0.15),
                quadrant_border: rgba_css(128, 128, 128, 0.1),
                quadrant_border_hover_dark: rgba_css(255, 255, 255, 0.2),
                quadrant_header_bg: rgba_css(128, 128, 128, 0.05),
                quadrant_header_border: rgba_css(128, 128, 128, 0.1),
                quadrant_count_bg: rgba_css(128, 128, 128, 0.1),
                quadrant_count_fg: p.text_tertiary,
                quadrant_title: p.text_secondary,
                calendar_line: p.border,
                nav_fg: p.text_secondary,
                nav_hover_fg: p.primary,
                nav_hover_bg: p.bg_tertiary,
                today_fg: p.text_secondary,
                today_bg: p.bg_secondary,
                today_border: p.border,
                today_hover_fg: p.primary,
                today_hover_bg: p.bg_tertiary,
                today_hover_border: p.primary,
            }
        }
    }
}

/// FAB：`0 4px 12px rgba(59,130,246,0.4)`，hover `0 6px 16px rgba(59,130,246,0.5)`
pub fn fab_shadow(hovered: bool) -> Vec<BoxShadow> {
    if hovered {
        vec![shadow(0., 6., 16., 0., rgba_css(59, 130, 246, 0.5))]
    } else {
        vec![shadow(0., 4., 12., 0., rgba_css(59, 130, 246, 0.4))]
    }
}

/// 主窗口尺寸 token
pub mod main_window {
    /// 标题栏按钮 28×28、图标 16
    pub const TITLE_BUTTON: f32 = 28.;
    pub const TITLE_ICON: f32 = 16.;
    /// FAB 48×48，距右下 20
    pub const FAB_SIZE: f32 = 48.;
    pub const FAB_INSET: f32 = 20.;
    /// 主列表滚动条宽 6；四象限 4
    pub const SCROLLBAR: f32 = 6.;
    pub const QUADRANT_SCROLLBAR: f32 = 4.;
    /// 日历待办条：高 20、间距 2、距格顶 28
    pub const BAR_HEIGHT: f32 = 20.;
    pub const BAR_GAP: f32 = 2.;
    pub const BAR_TOP_OFFSET: f32 = 28.;
}

// ---- 设置窗口（ui-settings-fence.md §2；Tailwind slate 系，与 `light()` 重合的直接取 `light()`） ----

/// 设置窗口专用颜色
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettingsTokens {
    /// 整窗底 `#F8FAFC`
    pub window_bg: Hsla,
    /// 标题栏 / 菜单 / 卡片 / 按钮白底
    pub surface: Hsla,
    /// 分隔线与卡片边框 `#E2E8F0`
    pub border: Hsla,
    /// 行之间的细线 / 菜单悬停底 `#F1F5F9`
    pub border_light: Hsla,
    /// 标题 `#1E293B`
    pub title: Hsla,
    /// 正文 / 标签 `#334155`
    pub label: Hsla,
    /// 次要文字 / 图标 `#64748B`
    pub secondary: Hsla,
    /// 说明文字 `#94A3B8`
    pub tertiary: Hsla,
    /// 应用主色 `#3B82F6` / 深 `#2563EB`
    pub primary: Hsla,
    pub primary_dark: Hsla,
    /// 菜单选中底 / 当前屏幕配置底 `#EFF6FF`
    pub active_bg: Hsla,
    /// 按钮悬停边框 `#CBD5E1`
    pub hover_border: Hsla,
    /// 危险按钮：字 `#DC2626`、边 `#FECACA`、悬停底 `#FEF2F2`、悬停边 `#FCA5A5`
    pub danger_text: Hsla,
    pub danger_border: Hsla,
    pub danger_hover_bg: Hsla,
    pub danger_hover_border: Hsla,
    /// http 明文警告：字 `#B45309`、图标 `#F59E0B`
    pub warning_text: Hsla,
    pub warning_icon: Hsla,
    /// 「高级」区：边 `#FDE68A`、底 `#FFFBEB`
    pub advanced_border: Hsla,
    pub advanced_bg: Hsla,
    /// 渐变（徽标、Logo）`#3B82F6 → #06B6D4`
    pub gradient_from: Hsla,
    pub gradient_to: Hsla,
    /// 新版本徽标：底 `#FEE2E2`、字 `#EF4444`
    pub update_badge_bg: Hsla,
    pub update_badge_text: Hsla,
}

static SETTINGS_TOKENS: LazyLock<SettingsTokens> = LazyLock::new(|| SettingsTokens {
    window_bg: hex(0xF8FAFC),
    surface: hex(0xFFFFFF),
    border: hex(0xE2E8F0),
    border_light: hex(0xF1F5F9),
    title: hex(0x1E293B),
    label: hex(0x334155),
    secondary: hex(0x64748B),
    tertiary: hex(0x94A3B8),
    primary: hex(0x3B82F6),
    primary_dark: hex(0x2563EB),
    active_bg: hex(0xEFF6FF),
    hover_border: hex(0xCBD5E1),
    danger_text: hex(0xDC2626),
    danger_border: hex(0xFECACA),
    danger_hover_bg: hex(0xFEF2F2),
    danger_hover_border: hex(0xFCA5A5),
    warning_text: hex(0xB45309),
    warning_icon: hex(0xF59E0B),
    advanced_border: hex(0xFDE68A),
    advanced_bg: hex(0xFFFBEB),
    gradient_from: hex(0x3B82F6),
    gradient_to: hex(0x06B6D4),
    update_badge_bg: hex(0xFEE2E2),
    update_badge_text: hex(0xEF4444),
});

/// 设置窗口颜色
pub fn settings_tokens() -> &'static SettingsTokens {
    &SETTINGS_TOKENS
}

// ---- 桌面收纳箱（ui-settings-fence.md §3、§4；深色半透明，白字） ----

/// 收纳箱窗口专用颜色
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FenceTokens {
    /// 文字 `#FFF`
    pub text: Hsla,
    /// 外框 `rgba(255,255,255,.14)`
    pub window_border: Hsla,
    /// 展开时标题栏下边线 `rgba(255,255,255,.08)`
    pub title_divider: Hsla,
    /// 标题文字阴影 `rgba(0,0,0,.4)`
    pub title_shadow: Hsla,
    /// 标题栏按钮 `rgba(255,255,255,.85)`、悬停底 `rgba(255,255,255,.14)`、已固定 `#60A5FA`
    pub button: Hsla,
    pub button_hover_bg: Hsla,
    pub button_active: Hsla,
    /// 空态 `rgba(255,255,255,.55)`
    pub empty: Hsla,
    /// 面板底 `rgba(15,23,42,.92)`、标签 `rgba(255,255,255,.7)`、数值 `rgba(255,255,255,.8)`
    pub panel_bg: Hsla,
    pub panel_label: Hsla,
    pub panel_value: Hsla,
    /// 色块边框 `rgba(255,255,255,.25)`，当前色 `#FFF`
    pub swatch_border: Hsla,
    /// 拖文件经过时的虚线框 `rgba(255,255,255,.45)`
    pub drop_outline: Hsla,
    /// 项目悬停底 `rgba(255,255,255,.12)`；选中底 `rgba(96,165,250,.32)` + 内描边 `rgba(147,197,253,.65)`
    pub item_hover: Hsla,
    pub item_selected: Hsla,
    pub item_selected_ring: Hsla,
    /// 名称文字阴影 `rgba(0,0,0,.65)`
    pub name_shadow: Hsla,
    /// 占位图标 `rgba(255,255,255,.75)`
    pub placeholder_icon: Hsla,
    /// 滚动条滑块 `rgba(255,255,255,.25)`
    pub scrollbar_thumb: Hsla,
    /// 右键菜单：底 `rgba(15,23,42,.96)`、边 `rgba(255,255,255,.12)`、项文字 `rgba(255,255,255,.9)`、
    /// 悬停 `rgba(255,255,255,.12)`、分隔线 `rgba(255,255,255,.1)`、附注 `rgba(255,255,255,.6)`、阴影 `rgba(0,0,0,.45)`
    pub menu_bg: Hsla,
    pub menu_border: Hsla,
    pub menu_text: Hsla,
    pub menu_hover: Hsla,
    pub menu_divider: Hsla,
    pub menu_note: Hsla,
    pub menu_shadow: Hsla,
}

static FENCE_TOKENS: LazyLock<FenceTokens> = LazyLock::new(|| FenceTokens {
    text: hex(0xFFFFFF),
    window_border: rgba_css(255, 255, 255, 0.14),
    title_divider: rgba_css(255, 255, 255, 0.08),
    title_shadow: rgba_css(0, 0, 0, 0.4),
    button: rgba_css(255, 255, 255, 0.85),
    button_hover_bg: rgba_css(255, 255, 255, 0.14),
    button_active: hex(0x60A5FA),
    empty: rgba_css(255, 255, 255, 0.55),
    panel_bg: rgba_css(15, 23, 42, 0.92),
    panel_label: rgba_css(255, 255, 255, 0.7),
    panel_value: rgba_css(255, 255, 255, 0.8),
    swatch_border: rgba_css(255, 255, 255, 0.25),
    drop_outline: rgba_css(255, 255, 255, 0.45),
    item_hover: rgba_css(255, 255, 255, 0.12),
    item_selected: rgba_css(96, 165, 250, 0.32),
    item_selected_ring: rgba_css(147, 197, 253, 0.65),
    name_shadow: rgba_css(0, 0, 0, 0.65),
    placeholder_icon: rgba_css(255, 255, 255, 0.75),
    scrollbar_thumb: rgba_css(255, 255, 255, 0.25),
    menu_bg: rgba_css(15, 23, 42, 0.96),
    menu_border: rgba_css(255, 255, 255, 0.12),
    menu_text: rgba_css(255, 255, 255, 0.9),
    menu_hover: rgba_css(255, 255, 255, 0.12),
    menu_divider: rgba_css(255, 255, 255, 0.1),
    menu_note: rgba_css(255, 255, 255, 0.6),
    menu_shadow: rgba_css(0, 0, 0, 0.45),
});

/// 收纳箱颜色
pub fn fence_tokens() -> &'static FenceTokens {
    &FENCE_TOKENS
}

/// 收纳箱字号 13px（`.fence-window`）
pub const FENCE_FONT_SIZE: Pixels = px(13.);

/// 「显示隐藏的图标」打开时，隐藏项（用户隐藏 + 以 `.` 开头的自动隐藏）的图标与名字整体不透明度
/// （同资源管理器里隐藏文件的半透明观感）
pub const FENCE_HIDDEN_ITEM_OPACITY: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_compare_case_insensitively() {
        assert!(is_same_color("#0f2942", "#0F2942"));
        assert!(is_same_color(" #000000 ", "#000000"));
        assert!(!is_same_color("#000000", "#1E293B"));
    }

    #[test]
    fn hex_channels_match_frontend_rules() {
        assert_eq!(hex_to_rgb_channels("#1E293B"), (0x1E, 0x29, 0x3B));
        assert_eq!(hex_to_rgb_channels("#abc"), (0xAA, 0xBB, 0xCC));
        assert_eq!(hex_to_rgb_channels("0F2942"), (0x0F, 0x29, 0x42));
        assert_eq!(hex_to_rgb_channels("#12345"), (0, 0, 0));
        assert_eq!(hex_to_rgb_channels("red"), (0, 0, 0));
        assert_eq!(hex_to_rgb_channels(""), (0, 0, 0));
    }

    #[test]
    fn dark_palette_uses_user_background() {
        let bg = WindowBackground {
            color: "#0F2942".into(),
            alpha: 0.6,
        };
        let dark = Palette::dark(&bg);
        assert!(dark.is_dark);
        assert_eq!(dark.window_bg, bg.to_hsla());
        assert!((dark.window_bg.a - 0.6).abs() < 1e-6);
        assert_eq!(dark.primary, light().primary);
        assert!(dark.shadow_md().is_empty());
        assert!(!light().shadow_md().is_empty());
    }

    #[test]
    fn todo_font_puts_user_family_first() {
        let font = todo_font("SimHei");
        assert_eq!(font.family.as_ref(), "SimHei");
        let fallbacks = font.fallbacks.unwrap();
        assert_eq!(fallbacks.fallback_list()[0], font_family().to_string());
        assert_eq!(todo_font("  ").family, font_family());
    }
}

// ---- 编辑 / 子任务 / 已完成 / 通知窗口与 Markdown（`ui-editor-windows.md`）----

/// 编辑类窗口与 Markdown 渲染的 token（`0xRRGGBB`，用 [`hex`] 取色；尺寸单位逻辑像素）
pub mod editor_theme {
    use gpui_kit::{Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, Hsla};

    // 编辑类窗口里直接写在 scoped CSS 里的 Tailwind slate / blue / red …
    /// slate-700：子任务标题、表单字段标签
    pub const SLATE_700: u32 = 0x334155;
    /// slate-300：虚线边框、复选框边
    pub const SLATE_300: u32 = 0xCBD5E1;
    /// 子任务面板底
    pub const PANEL_BG: u32 = 0xFAFBFC;
    /// blue-50：小图标按钮 hover 底
    pub const BLUE_50: u32 = 0xEFF6FF;
    /// blue-100 / blue-200：已完成窗口"恢复"按钮底 / hover 底
    pub const BLUE_100: u32 = 0xDBEAFE;
    pub const BLUE_200: u32 = 0xBFDBFE;
    /// blue-300：搜索框聚焦边
    pub const BLUE_300: u32 = 0x93C5FD;
    /// red-100 / red-200 / red-600：删除按钮底 / hover 底 / hover 字
    pub const RED_100: u32 = 0xFEE2E2;
    pub const RED_200: u32 = 0xFECACA;
    pub const RED_600: u32 = 0xDC2626;
    /// indigo-100 / indigo-500：子任务"查看"按钮 hover
    pub const INDIGO_100: u32 = 0xE0E7FF;
    pub const INDIGO_500: u32 = 0x6366F1;
    /// cyan-500：渐变终点（进度条、勾选、通知图标）
    pub const CYAN_500: u32 = 0x06B6D4;
    /// sky-50 / sky-100 / sky-700：子任务进度卡片渐变与数字
    pub const SKY_50: u32 = 0xF0F9FF;
    pub const SKY_100: u32 = 0xE0F2FE;
    pub const SKY_700: u32 = 0x0369A1;
    /// amber-50 / amber-700：暂存子任务提示条
    pub const AMBER_50: u32 = 0xFFFBEB;
    pub const AMBER_700: u32 = 0xB45309;

    // ---- Markdown（Milkdown theme-nord + MarkdownEditor.vue scoped 的实际生效值，已按参考截图校准）----
    /// 正文字色
    pub const MD_TEXT: u32 = 0x1E293B;
    /// 列表标记、行内代码字色（nord10）
    pub const MD_ACCENT: u32 = 0x5E81AC;
    /// 行内代码底
    pub const MD_CODE_BG: u32 = 0xF1F5F9;
    /// 代码块底 / 字
    pub const MD_PRE_BG: u32 = 0x1E293B;
    pub const MD_PRE_TEXT: u32 = 0xE2E8F0;
    /// 引用左线 / 字
    pub const MD_QUOTE_BORDER: u32 = 0x3B82F6;
    pub const MD_QUOTE_TEXT: u32 = 0x64748B;
    /// 表格内线、分隔线
    pub const MD_RULE: u32 = 0xE2E8F0;
    /// 表头底；表体奇数列底（gray-50）
    pub const MD_TH_BG: u32 = 0xF8FAFC;
    pub const MD_TD_ODD_BG: u32 = 0xF9FAFB;
    /// 任务项方框边
    pub const MD_TASK_BORDER: u32 = 0x94A3B8;

    /// 段落 16px / 行高 1.6，上下外边距 0.4em
    pub const MD_P_SIZE: f32 = 16.;
    pub const MD_P_LINE: f32 = 25.6;
    pub const MD_P_MARGIN: f32 = 6.4;
    /// 标题 (字号, 行高倍数, 上外边距, 下外边距)，h1..h6
    pub const MD_HEADINGS: [(f32, f32, f32, f32); 6] = [
        (30., 1.25, 18., 9.),
        (24., 1.333, 14.4, 7.2),
        (20., 1.4, 12., 6.),
        (16., 1.5, 12., 0.),
        (14., 1.4286, 8., 0.),
        (14., 1.4286, 8., 0.),
    ];
    /// 列表缩进 / 外边距 / 项下边距
    pub const MD_LIST_INDENT: f32 = 24.;
    pub const MD_LIST_MARGIN: f32 = 5.6;
    pub const MD_LI_MARGIN: f32 = 4.;
    /// 列表标记字号（li 继承编辑器根 14px）
    pub const MD_MARKER_SIZE: f32 = 14.;
    /// 引用外边距（0.5em × 14px）
    pub const MD_QUOTE_MARGIN: f32 = 7.;
    /// 代码块 14px / 20px，内边距 12 16，圆角 8
    pub const MD_PRE_SIZE: f32 = 14.;
    pub const MD_PRE_LINE: f32 = 20.;
    /// 表格外边距 0.6em（14px）；单元格内边距 12 24；圆角 8
    pub const MD_TABLE_MARGIN: f32 = 8.4;
    /// 分隔线外边距 1em（14px）
    pub const MD_HR_MARGIN: f32 = 14.;

    /// 等宽字体：Windows Consolas（Milkdown 等宽栈在 Windows 上先命中它）；Linux 用 Liberation Mono（截图替身）
    pub fn mono_font() -> Font {
        let (family, fallbacks): (&str, Vec<String>) = if cfg!(windows) {
            (
                "Consolas",
                vec!["Courier New".into(), "Microsoft YaHei".into()],
            )
        } else {
            ("Liberation Mono", vec!["Noto Sans CJK SC".into()])
        };
        Font {
            family: family.to_string().into(),
            features: FontFeatures::default(),
            fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }

    /// 衬线字体（引用块 `ui-serif, Georgia, Cambria, "Times New Roman", serif`；中文在 Windows 上回落宋体）
    pub fn serif_font() -> Font {
        let (family, fallbacks): (&str, Vec<String>) = if cfg!(windows) {
            ("Georgia", vec!["Cambria".into(), "SimSun".into()])
        } else {
            ("Liberation Serif", vec!["Noto Serif CJK SC".into()])
        };
        Font {
            family: family.to_string().into(),
            features: FontFeatures::default(),
            fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }

    /// `color` 以 `alpha` 不透明度叠在白底上的等效不透明色（GPUI 的外阴影会透过半透明背景显示出来，
    /// CSS 不会；带阴影的浅色块用它代替半透明背景）
    pub fn tint(color: Hsla, alpha: f32) -> Hsla {
        let c = color.to_rgb();
        let mix = |v: f32| v * alpha + (1. - alpha);
        gpui_kit::Rgba {
            r: mix(c.r),
            g: mix(c.g),
            b: mix(c.b),
            a: 1.,
        }
        .into()
    }
}
