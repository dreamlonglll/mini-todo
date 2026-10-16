//! 进程级服务（与具体窗口无关）。
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`logging`] | 日志文件（2MB 轮转、保留 1 份、本地时间），路径同旧版（mac：`~/Library/Logs/com.tauri-app.mini-todo/`） |
//! | [`single_instance`] | 单实例（Windows 命名互斥量 + `WM_COPYDATA`；macOS Unix socket；唤起已运行实例，与旧版 Tauri 插件互认） |
//! | [`sync`] | 同步编排 `run_sync`（标题栏按钮 / 自动同步 / 设置页共用）与 SyncReport 提示文案 |
//! | [`auto_sync`] | 自动同步定时器（间隔轮询、失败段只提示一次；设置窗口关闭后重启） |
//! | [`autostart`] | 开机自启（Windows 注册表 Run 键 / macOS LaunchAgent，与旧版同名同参数；托盘勾选与设置页共用） |
//! | [`tray`] | 系统托盘 / 菜单栏图标（Windows、macOS：菜单、单击唤起去抖、固定模式勾选；收纳箱三个入口仅 Windows） |
//! | [`scheduler`] | 提醒调度（对齐整分钟、启动补发重复提醒，按通知方式投递） |
//! | [`notify`] | 系统通知（Windows Toast，AppUserModelID 与旧版一致；macOS `UNUserNotificationCenter`，归属 bundle id） |
//! | [`window_mode`] | 主窗口固定 / 嵌入桌面 / 贴边隐藏、位置与屏幕配置记忆、托盘唤起、重置位置、200ms 轮询 |
//! | [`update_check`] | GitHub 版本检查（启动检查 + 设置「关于 → 检查更新」共用） |
//! | [`fences`] | 桌面收纳箱生命周期：恢复 / 新建 / 显示隐藏 / 重置位置、目录监听、嵌入守护、屏幕组合检测 |
//!
//! 启动顺序见 `lib.rs::run`：`tray::start` → `auto_sync::start` → `scheduler::start` → `fences` 恢复。

pub mod auto_sync;
pub mod autostart;
pub mod fences;
pub mod logging;
pub mod notify;
pub mod scheduler;
pub mod single_instance;
pub mod sync;
pub mod tray;
pub mod update_check;
pub mod window_mode;
