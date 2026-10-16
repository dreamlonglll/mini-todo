//! macOS 菜单栏图标：单色模板图标 `assets/tray-template.png`（36×36 = 18pt@2x，黑色字形 + alpha；
//! 生成脚本 `pc-gpui/scripts/macos/gen-tray-icon.py`）。
//!
//! tray-icon 在 mac 上把图标高度固定为 18pt、宽度按比例（`platform_impl/macos/mod.rs`），并按
//! `with_icon_as_template(true)` 设 `NSImage.template`：系统只取 alpha，随菜单栏深浅色自动着色。
//! Windows 用 exe 资源里的彩色图标（`Icon::from_resource`，仅 Windows 有），不走这里。
//!
//! 不依赖 GPUI：`pc-gpui/scripts/mac-check` 按 `aarch64-apple-darwin` 交叉检查本文件。

use tray_icon::Icon;

/// 嵌进二进制的模板图标 PNG
const TEMPLATE_PNG: &[u8] = include_bytes!("../../../assets/tray-template.png");

/// 解码模板图标（调用方再 `with_icon_as_template(true)`）
pub fn menu_bar_icon() -> Result<Icon, String> {
    let image = image::load_from_memory_with_format(TEMPLATE_PNG, image::ImageFormat::Png)
        .map_err(|e| format!("解码菜单栏图标失败: {e}"))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).map_err(|e| e.to_string())
}
