//! 系统托盘（Windows：`tray-icon` + 其自带的 muda 菜单；旧版 `lib.rs` L256-426）。**W1 实现**。
//!
//! 菜单（旧版顺序与文案）：添加待办项、（Windows）新增收纳箱 / 显示/隐藏所有收纳箱 / 重置收纳箱位置、分隔、
//! 固定模式（勾选）、重置位置、打开设置、开机自启动（勾选）、分隔、退出。左键单击 300ms 去抖后唤起主窗口；
//! 开发隔离时 tooltip 为 `Mini Todo（开发版）`。图标可直接 `tray_icon::Icon::from_resource(1, None)`（exe 资源 ID 1）。
//! 托盘 / 菜单事件在窗口过程里触发：**不要**在回调里碰 GPUI `App`，经 channel 投递到前台任务处理。
//! 收纳箱的三个菜单项调用 W3 在 `windows::fence` 里提供的入口。

use gpui_kit::App;

/// 启动托盘（`run()` 里调用；非 Windows 什么也不做）
pub fn start(_cx: &mut App) {}
