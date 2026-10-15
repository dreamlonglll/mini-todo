//! 纯工具函数（旧前端 `src/utils/*.ts` 的 1:1 移植，带原 vitest 用例）。**W2**。
//!
//! | 模块 | 旧文件 | 内容 |
//! |---|---|---|
//! | [`datetime`] | `datetime.ts` | 时间解析 / 规范化 / 拆分合成 / 显示格式（K1） |
//! | [`image_ref`] | `imageRef.ts` | `minitodo-image://` 引用、旧 asset URL 兼容、图片扩展名（K5） |
//! | [`file_link`] | `fileLink.ts` | 链接协议白名单（http/https/mailto、file:/// 定位、其余拦截） |
//! | [`editor_shortcuts`] | `editorShortcuts.ts` | 编辑窗口 Esc / Ctrl+Enter |

pub mod datetime;
pub mod editor_shortcuts;
pub mod file_link;
pub mod image_ref;
