//! 跨模块共享的小工具。
//!
//! 当前只有 `id_string`：从 `serde_json::Value` 的 `"id"` 字段提取字符串形式的
//! id。PC 端 todo / subtask 的 `id` 列是 SQLite `INTEGER PRIMARY KEY AUTOINCREMENT`
//! (即 i64)；云端把它统一转字符串作为 KV-style PK 使用。

use serde_json::Value;

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

#[cfg(test)]
mod tests {
    use super::*;
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
}
