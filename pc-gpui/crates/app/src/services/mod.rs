//! 进程级服务（与具体窗口无关）。
//!
//! | 模块 | 内容 | 负责 |
//! |---|---|---|
//! | [`logging`] | 日志文件（2MB 轮转、保留 1 份、本地时间），路径同旧版 | 基础框架 |
//! | [`single_instance`] | 单实例（Windows 命名互斥量 + 唤起已运行实例） | 基础框架 |
//! | [`sync`] | 同步编排 `run_sync` 与 SyncReport 提示文案 | 基础框架（自动同步定时器：W1） |
//! | [`autostart`] | 开机自启（注册表 Run 键，与旧版同名同参数） | 基础框架（托盘勾选：W1；设置页：W3） |
//! | [`tray`] | 托盘 | W1（现为空实现） |
//! | [`auto_sync`] | 自动同步定时器 | W1（现为空实现） |
//! | [`scheduler`] | 提醒调度与系统通知 | W2（现为空实现） |
//! | [`window_mode`] | 主窗口固定 / 嵌入桌面 / 贴边隐藏、位置记忆、托盘唤起、重置 | W1 |
//! | [`update_check`] | GitHub 版本检查 | W1 |
//! | [`fences`] | 桌面收纳箱生命周期：恢复 / 新建 / 显示隐藏 / 重置位置、目录监听、嵌入守护、屏幕组合检测 | W3 |

pub mod auto_sync;
pub mod autostart;
pub mod fences;
pub mod logging;
pub mod scheduler;
pub mod single_instance;
pub mod sync;
pub mod tray;
// ---- W1 追加 ----
pub mod update_check;
pub mod window_mode;
