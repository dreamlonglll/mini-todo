//! 把 app / core 里不依赖 GPUI（core 的则不依赖 core 其它模块）的 mac 专属源文件原样编进来，在 Linux / Windows 上 `--target aarch64-apple-darwin` 做类型检查。
//! 用法见 `Cargo.toml` 顶部注释。每加一个这样的源文件，就在下面加一行 `#[path]` 模块。
#![cfg(target_os = "macos")]
#![allow(dead_code)]

// 常驻与身份：开机自启（LaunchAgent）、单实例 Unix socket、菜单栏模板图标
#[path = "../../../crates/app/src/services/autostart.rs"]
mod autostart;
#[path = "../../../crates/app/src/services/single_instance/unix_socket.rs"]
mod single_instance_unix_socket;
#[path = "../../../crates/app/src/services/tray/mac_icon.rs"]
mod tray_mac_icon;

// 窗口平台层：NSWindow / NSScreen / NSEvent 的原始调用
#[path = "../../../crates/app/src/platform/macos/ns.rs"]
mod platform_macos_ns;

// core 的收纳箱文件层：移动（renamex_np + NSFileManager）、图标、打开 / 在访达中显示、iCloud 判定
#[path = "../../../crates/core/src/services/fence_fs/mac.rs"]
mod core_fence_fs_mac;
