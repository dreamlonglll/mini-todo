//! 资源：Element Plus 图标（`assets/icons/ep/*.svg`）与应用自绘图标（`assets/icons/app/*.svg`），
//! `include_bytes!` 编进二进制；其它路径交给 gpui-kit 自带的 [`gpui_kit::assets::Assets`]
//! （gpui-component 默认的 106 个 Lucide 图标）。
//!
//! 新增图标：把 SVG（`fill="currentColor"`，kebab-case 文件名）放进对应目录，再在下面的
//! [`ep_icons!`] / [`app_icons!`] 表里加一行即可（枚举变体、路径、嵌入表同时生成）。

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

/// 生成图标枚举、`path()` 与嵌入表
macro_rules! icon_table {
    ($enum_name:ident, $dir:literal, $table:ident, { $($(#[$meta:meta])* $variant:ident => $file:literal),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $enum_name {
            $($(#[$meta])* $variant),*
        }

        impl $enum_name {
            /// 全部变体（测试与调试用）
            pub const ALL: &'static [$enum_name] = &[$($enum_name::$variant),*];

            /// 资源路径（`svg().path(..)` / `AssetSource::load` 用）
            pub const fn path(self) -> &'static str {
                match self {
                    $($enum_name::$variant => concat!("icons/", $dir, "/", $file, ".svg")),*
                }
            }
        }

        const $table: &[(&str, &[u8])] = &[
            $((
                concat!("icons/", $dir, "/", $file, ".svg"),
                include_bytes!(concat!("../assets/icons/", $dir, "/", $file, ".svg")),
            )),*
        ];
    };
}

// Element Plus 2.3.x 图标（@element-plus/icons-vue，MIT），文件名与官方 kebab-case 一致。
// 前 54 个是旧前端直接 import 的（含 ElMessage 用的 CircleCloseFilled），后 7 个是 Element Plus
// 组件内部用到的（可清空输入框、日期面板、数字输入框、图片预览）。
icon_table!(EpIcon, "ep", EP_ICONS, {
    ArrowDown => "arrow-down",
    ArrowLeft => "arrow-left",
    ArrowRight => "arrow-right",
    ArrowUp => "arrow-up",
    Back => "back",
    Bell => "bell",
    Brush => "brush",
    Calendar => "calendar",
    Check => "check",
    CircleCheck => "circle-check",
    CirclePlus => "circle-plus",
    Clock => "clock",
    Close => "close",
    Connection => "connection",
    Delete => "delete",
    Document => "document",
    Download => "download",
    Edit => "edit",
    EditPen => "edit-pen",
    Expand => "expand",
    Finished => "finished",
    Fold => "fold",
    Folder => "folder",
    FolderOpened => "folder-opened",
    FullScreen => "full-screen",
    Hide => "hide",
    InfoFilled => "info-filled",
    List => "list",
    Loading => "loading",
    Lock => "lock",
    Monitor => "monitor",
    Moon => "moon",
    MoreFilled => "more-filled",
    Plus => "plus",
    Promotion => "promotion",
    Rank => "rank",
    Refresh => "refresh",
    RefreshLeft => "refresh-left",
    RefreshRight => "refresh-right",
    Right => "right",
    Search => "search",
    Select => "select",
    Setting => "setting",
    SuccessFilled => "success-filled",
    Sunny => "sunny",
    Timer => "timer",
    Top => "top",
    TopRight => "top-right",
    Unlock => "unlock",
    Upload => "upload",
    View => "view",
    Warning => "warning",
    WarningFilled => "warning-filled",
    CircleCloseFilled => "circle-close-filled",
    // Element Plus 组件内部图标
    CircleClose => "circle-close",
    DArrowLeft => "d-arrow-left",
    DArrowRight => "d-arrow-right",
    Minus => "minus",
    ZoomIn => "zoom-in",
    ZoomOut => "zoom-out",
    ScaleToOriginal => "scale-to-original",
});

// 旧前端里的内联 SVG 与自绘小图形
icon_table!(AppIcon, "app", APP_ICONS, {
    /// 标题栏列表模式下的视图切换图标（四方格，TitleBar.vue 内联 SVG，viewBox 24）
    QuadGrid => "quad-grid",
    /// 已完成窗口的关闭按钮（CompletedView.vue 内联 SVG：10×10 描边 ✕）
    CloseThin => "close-thin",
    /// Element Plus 深色 tooltip 的箭头（宽 10、高 5 的三角形，尖端朝下 / 朝上）
    ArrowDownTip => "arrow-down-tip",
    ArrowUpTip => "arrow-up-tip",
});

fn embedded(path: &str) -> Option<&'static [u8]> {
    EP_ICONS
        .iter()
        .chain(APP_ICONS)
        .find(|(p, _)| *p == path)
        .map(|(_, bytes)| *bytes)
}

/// 应用资源源：先查自带图标表，再交给 gpui-kit 默认资源（gpui-component 的 Lucide 图标等）
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = embedded(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        // 注意：gpui-kit 的 Assets 对缺失路径返回 Err（不是 Ok(None)）
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out: Vec<SharedString> = EP_ICONS
            .iter()
            .chain(APP_ICONS)
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect();
        out.extend(gpui_kit::assets::Assets.list(path).unwrap_or_default());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_path_is_embedded_and_is_svg() {
        for icon in EpIcon::ALL {
            let bytes = AppAssets
                .load(icon.path())
                .unwrap()
                .unwrap_or_else(|| panic!("{}", icon.path()));
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("<svg"), "{} 不是 SVG", icon.path());
            assert!(
                text.contains("currentColor"),
                "{} 没用 currentColor",
                icon.path()
            );
        }
        for icon in AppIcon::ALL {
            let bytes = AppAssets
                .load(icon.path())
                .unwrap()
                .unwrap_or_else(|| panic!("{}", icon.path()));
            assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
        }
    }

    #[test]
    fn ep_icon_paths_are_kebab_case() {
        assert_eq!(
            EpIcon::CircleCloseFilled.path(),
            "icons/ep/circle-close-filled.svg"
        );
        assert_eq!(EpIcon::FolderOpened.path(), "icons/ep/folder-opened.svg");
        assert_eq!(AppIcon::QuadGrid.path(), "icons/app/quad-grid.svg");
    }

    #[test]
    fn list_includes_own_icons() {
        let listed = AppAssets.list("icons/ep/").unwrap();
        assert!(listed.iter().any(|p| p.as_ref() == "icons/ep/plus.svg"));
    }
}
