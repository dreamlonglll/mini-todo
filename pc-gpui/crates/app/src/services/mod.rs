//! 进程级服务（与具体窗口无关）。
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`logging`] | 日志文件（2MB 轮转、保留 1 份、本地时间），路径同旧版（mac：`~/Library/Logs/com.tauri-app.mini-todo/`） |
//! | [`single_instance`] | 单实例（Windows 命名互斥量 + `WM_COPYDATA`；macOS Unix socket；唤起已运行实例，与旧版 Tauri 插件互认） |
//! | [`sync`] | 同步编排 `run_sync`（标题栏按钮 / 自动同步 / 设置页共用）与 SyncReport 提示文案 |
//! | [`auto_sync`] | 自动同步定时器（间隔轮询、失败段只提示一次；设置窗口关闭后重启） |
//! | [`autostart`] | 开机自启（Windows 注册表 Run 键 / macOS LaunchAgent，与旧版同名同参数；托盘勾选与设置页共用） |
//! | [`tray`] | 系统托盘 / 菜单栏图标（Windows、macOS：菜单、单击唤起去抖、显示日历 / 开机自启动勾选、收纳箱三个入口） |
//! | [`scheduler`] | 提醒调度（对齐整分钟、启动补发重复提醒，按通知方式投递） |
//! | [`notify`] | 系统通知（Windows Toast，AppUserModelID 与旧版一致；macOS `UNUserNotificationCenter`，归属 bundle id） |
//! | [`notice`] | 整个应用的提示（自动同步失败、加载待办失败、导入成功……）显示在看得见的清单 / 日历窗口里；都看不见时先存着，下次有窗口被激活时补上 |
//! | [`block_windows`] | 桌面块窗口（清单窗口 / 日历窗口）的生命周期：启动开窗、关闭 = 隐藏、唤起时显示隐藏的清单窗口、同步来的清单开 / 关窗、「显示日历」、5 秒变更轮询 |
//! | [`window_mode`] | 桌面块窗口按窗口的锁定 / 嵌入桌面 / 贴边隐藏、几何与锁定的持久化（`store`，core 的窗口布局表）、唤起所有窗口、重置位置、200ms 轮询 |
//! | [`update_check`] | GitHub 版本检查（启动检查 + 设置「关于 → 检查更新」共用） |
//! | [`fences`] | 桌面收纳箱生命周期：恢复 / 新建 / 显示隐藏 / 重置位置、目录监听、嵌入守护（Windows）/ 桌面层（macOS）、屏幕组合检测 |
//! | [`snap`] | 吸附组（清单窗口 / 日历窗口 / 收纳箱）的跨窗口运行时：参与者登记、拖动开始时的目标快照、目标窗口上的高亮 |
//! | [`mac_menu`] | macOS 应用菜单（⌘Q / ⌘, / ⌘H / ⌘W / ⌘M、编辑菜单）与 Dock 唤起（全平台编译，只在 macOS 上调用） |
//!
//! 启动顺序见 `lib.rs::run`：`block_windows::start`（清单 / 日历窗口）→ `tray::start` → `auto_sync::start` → `scheduler::start` →
//! `fences` 恢复。

pub mod auto_sync;
pub mod autostart;
pub mod block_windows;
pub mod fences;
pub mod logging;
pub mod mac_menu;
pub mod notice;
pub mod notify;
pub mod scheduler;
pub mod single_instance;
pub mod snap;
pub mod sync;
pub mod tray;
pub mod update_check;
pub mod window_mode;
