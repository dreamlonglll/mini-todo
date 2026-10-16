//! mini-todo 业务核心：GPUI 版 PC 端（`pc-gpui/crates/app`）与界面无关的全部逻辑。
//!
//! 由旧 Tauri 版（`src-tauri/src`，3.0 起已删除，见 git 历史）复制而来：`db/**`、`services::{webdav, secret, fence_fs}`
//! 原样复制；`commands/*.rs` 去掉 Tauri 外壳（`#[tauri::command]` / `State` / `AppHandle` / 事件）后成为普通的
//! **阻塞**函数。不依赖 GPUI / Tauri / tokio，app 层在后台线程调用它们，再把结果交回界面。
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`db`] | SQLite 连接、迁移 v1~v30、模型、设置键值、同步存储、时间格式、路径、备份 |
//! | [`todo`](mod@todo) | 待办 / 子任务 CRUD、排序、图片保存、从文件导入子任务 |
//! | [`data`] | 导入导出（ZIP / JSON，格式 4.0）、应用设置块 |
//! | [`sync`] | WebDAV 同步（智能同步 / 强制拉取 / 强制推送）与同步设置 |
//! | [`settings`] | 单项设置、运行时偏好、屏幕组合 ID、屏幕配置 |
//! | [`holiday`] | 法定节假日（网络 + 本地缓存） |
//! | [`lunar`] | 农历 / 节气 / 传统节日 |
//! | [`reminder`] | 提醒调度纯逻辑、应用内通知窗口摆放 |
//! | [`window_logic`] | 贴边隐藏状态机、桌面模式守护判定、托盘去抖 |
//! | [`fence`] | 桌面收纳箱元数据 / 几何 / 内容（文件层在 [`services::fence_fs`]） |
//!
//! 公开 API 与对应的旧 Tauri 命令见 crate 目录下的 `README.md`。

pub mod data;
pub mod db;
pub mod fence;
pub mod holiday;
pub mod lunar;
pub mod reminder;
pub mod services;
pub mod settings;
pub mod sync;
pub mod todo;
pub mod window_logic;

pub use db::Database;
