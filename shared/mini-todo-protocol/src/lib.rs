//! PC（`pc-gpui`）与云端（`cloud`）共用的跨端同步协议工具。
//!
//! 统一维护两端必须逐字一致的规则，避免改一端忘另一端：
//! - [`ids`]：JS 安全整数范围内的分布式记录 ID 生成（UNIX 毫秒 × 1000 + 随机后缀）
//! - [`image_name`]：K5 安全图片文件名校验、白名单扩展名与 Content-Type 映射表
//! - [`webdav_xml`]：WebDAV PROPFIND multistatus 解析与 ETag 强弱处理
//! - [`crypto`]：可选的客户端端到端加密（PBKDF2-HMAC-SHA256 + ChaCha20-Poly1305）

pub mod crypto;
pub mod ids;
pub mod image_name;
pub mod webdav_xml;
