//! 跨模块共享的小工具。
//!
//! - `id_string`：从 `serde_json::Value` 的 `"id"` 字段提取字符串形式的 id。PC 端
//!   todo / subtask 的 `id` 列是 SQLite `INTEGER PRIMARY KEY AUTOINCREMENT`（即 i64）；
//!   云端把它统一转字符串作为 KV-style PK 使用。
//! - `merge_json_shallow`：PATCH 的浅合并（todos / subtasks 共用）。
//! - `public_error_message`：给客户端看的错误消息（本地存储错误脱敏）。
//! - 图片扩展名 ↔ Content-Type 的唯一一张映射表（API 下发与 WebDAV 上传共用）。

use serde_json::{Map, Value};

/// 从 `Value` 中提取 `"id"` 字段并转字符串。
///
/// - i64：直接 to_string
/// - string：原样返回（空字符串视为缺失 → None，避免空 id 进 PK）
/// - 其他类型 / 缺失：None
pub fn id_string(v: &Value) -> Option<String> {
    let raw = v.get("id")?;
    if let Some(n) = raw.as_i64() {
        return Some(n.to_string());
    }
    if let Some(s) = raw.as_str() {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

/// 浅合并：把 `patch` 的顶层字段覆盖到 `target`（`null` 也写入，表示显式置空）。
/// 调用方负责先校验 `patch`（K6：只含允许的字段，不含 `id` 等服务端字段）。
pub fn merge_json_shallow(target: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (k, v) in patch {
        target.insert(k.clone(), v.clone());
    }
}

/// 本地存储错误（SQLite / 文件系统）在对外消息里的替代文字。
pub const LOCAL_STORAGE_ERROR: &str = "本地存储错误（详情见服务端日志）";

/// 给客户端（`/sync*` 响应、`/health` 的 lastPullError / lastPushError）看的错误消息。
///
/// 按 anyhow 错误链逐层输出上下文；遇到 `rusqlite::Error` / `std::io::Error` 就换成
/// 通用文字并停止——原始 SQLite 错误会暴露表结构 / SQL，IO 错误常带服务端文件路径。
/// WebDAV / 网络层面的错误（状态码、连接失败）原样保留，便于用户排查。完整错误链由
/// 调用方写进服务端日志。
pub fn public_error_message(err: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for cause in err.chain() {
        if cause.is::<rusqlite::Error>() || cause.is::<std::io::Error>() {
            parts.push(LOCAL_STORAGE_ERROR.to_string());
            break;
        }
        parts.push(cause.to_string());
    }
    parts.join(": ")
}

/// 允许的图片扩展名与对应的 Content-Type（K5 白名单：png / jpg / jpeg / webp / gif / bmp）。
/// svg 故意不在表内：可内嵌脚本，内联渲染是存储型 XSS 向量。
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
    ("bmp", "image/bmp"),
];

/// 文件名的小写扩展名。
pub fn extension_of(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// 按扩展名得到图片 Content-Type；不在白名单内（含 svg）一律 `application/octet-stream`。
pub fn image_content_type(name: &str) -> &'static str {
    let ext = extension_of(name);
    IMAGE_TYPES
        .iter()
        .find(|(e, _)| Some(*e) == ext.as_deref())
        .map(|(_, ct)| *ct)
        .unwrap_or("application/octet-stream")
}

/// 白名单扩展名（大小写不敏感）→ 规范小写扩展名；其它返回 `None`。
pub fn allowed_image_ext(ext: &str) -> Option<&'static str> {
    let lower = ext.to_ascii_lowercase();
    IMAGE_TYPES
        .iter()
        .find(|(e, _)| *e == lower)
        .map(|(e, _)| *e)
}

/// Content-Type → 扩展名（上传时客户端没给文件名用）。`image/jpeg` 映射到 `jpg`。
pub fn image_ext_for_content_type(ct: &str) -> Option<&'static str> {
    let essence = ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if essence == "image/jpg" {
        return Some("jpg");
    }
    IMAGE_TYPES
        .iter()
        .find(|(_, t)| *t == essence)
        .map(|(e, _)| *e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use serde_json::json;

    #[test]
    fn numeric_id() {
        assert_eq!(id_string(&json!({"id": 42})), Some("42".to_string()));
    }

    #[test]
    fn string_id() {
        assert_eq!(id_string(&json!({"id": "abc"})), Some("abc".to_string()));
    }

    #[test]
    fn empty_string_id_treated_as_missing() {
        assert_eq!(id_string(&json!({"id": ""})), None);
    }

    #[test]
    fn missing_field() {
        assert_eq!(id_string(&json!({})), None);
    }

    #[test]
    fn null_or_bool_treated_as_missing() {
        assert_eq!(id_string(&json!({"id": null})), None);
        assert_eq!(id_string(&json!({"id": true})), None);
    }

    #[test]
    fn shallow_merge_overwrites_and_writes_null() {
        let mut t = json!({"a": 1, "b": "x"}).as_object().cloned().unwrap();
        let p = json!({"b": null, "c": [1]}).as_object().cloned().unwrap();
        merge_json_shallow(&mut t, &p);
        assert_eq!(Value::Object(t), json!({"a": 1, "b": null, "c": [1]}));
    }

    #[test]
    fn public_message_hides_sqlite_and_io_details() {
        let sqlite = rusqlite::Connection::open_in_memory()
            .unwrap()
            .execute("SELECT * FROM no_such_table", [])
            .unwrap_err();
        let e = Err::<(), _>(sqlite)
            .context("合并远端 sync-data 失败")
            .unwrap_err();
        let msg = public_error_message(&e);
        assert_eq!(
            msg,
            format!("合并远端 sync-data 失败: {}", LOCAL_STORAGE_ERROR)
        );
        assert!(!msg.contains("no_such_table"));

        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/var/lib/minitodo/x");
        let e = anyhow::Error::new(io).context("写图片失败");
        let msg = public_error_message(&e);
        assert!(!msg.contains("/var/lib"), "{}", msg);

        // 网络 / WebDAV 层面的错误原样保留
        let e = anyhow::anyhow!("WebDAV PUT /mini-todo/sync-data.json.gz 返回状态 507");
        assert_eq!(
            public_error_message(&e),
            "WebDAV PUT /mini-todo/sync-data.json.gz 返回状态 507"
        );
    }

    #[test]
    fn image_type_table() {
        assert_eq!(image_content_type("a.PNG"), "image/png");
        assert_eq!(image_content_type("a.jpeg"), "image/jpeg");
        assert_eq!(image_content_type("a.svg"), "application/octet-stream");
        assert_eq!(image_content_type("a.unknown"), "application/octet-stream");
        assert_eq!(image_content_type("noext"), "application/octet-stream");
        assert_eq!(allowed_image_ext("JPG"), Some("jpg"));
        assert_eq!(allowed_image_ext("svg"), None);
        assert_eq!(allowed_image_ext(".."), None);
        assert_eq!(image_ext_for_content_type("image/png"), Some("png"));
        assert_eq!(image_ext_for_content_type("image/JPEG"), Some("jpg"));
        assert_eq!(image_ext_for_content_type("image/jpg"), Some("jpg"));
        assert_eq!(
            image_ext_for_content_type("image/webp; charset=binary"),
            Some("webp")
        );
        assert_eq!(image_ext_for_content_type("image/svg+xml"), None);
        assert_eq!(image_ext_for_content_type("text/plain"), None);
    }
}
