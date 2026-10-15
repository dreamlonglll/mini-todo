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

pub mod auto_sync;
pub mod autostart;
pub mod logging;
pub mod scheduler;
pub mod single_instance;
pub mod sync;
pub mod tray;
// ---- W2 ----
/// 系统通知（notify-rust）
pub mod notify;
