//! 跨端图片命名校验与 MIME 映射（契约 K5）。
//!
//! - 安全图片文件名：单个普通路径段，`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$` 且不含 `..`。
//! - 允许的图片扩展名白名单：`png` / `jpg` / `jpeg` / `webp` / `gif` / `bmp`（故意排除 `svg`）。

use std::path::Path;

/// 图片文件名最大长度（K5）
pub const MAX_IMAGE_NAME_LEN: usize = 128;

/// 允许的图片扩展名与对应 Content-Type（K5 白名单；svg 故意不在表内防存储型 XSS）
pub const IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
    ("bmp", "image/bmp"),
];

/// K5 安全图片文件名校验
pub fn is_safe_image_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_NAME_LEN {
        return false;
    }
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    if !bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return false;
    }
    !name.contains("..")
}

/// 提取文件名的小写扩展名
pub fn extension_of(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// 按文件名扩展名返回 Content-Type；不在白名单内（含 svg）一律 `application/octet-stream`
pub fn image_content_type(name: &str) -> &'static str {
    let ext = extension_of(name);
    IMAGE_TYPES
        .iter()
        .find(|(e, _)| Some(*e) == ext.as_deref())
        .map(|(_, ct)| *ct)
        .unwrap_or("application/octet-stream")
}

/// 白名单扩展名（大小写不敏感，不带前导点）→ 规范小写扩展名
pub fn allowed_image_ext(ext: &str) -> Option<&'static str> {
    let lower = ext.to_ascii_lowercase();
    IMAGE_TYPES
        .iter()
        .find(|(e, _)| *e == lower)
        .map(|(e, _)| *e)
}

/// 规范化用户输入/请求头里的扩展名（去除首尾空白与可选前导 `.`，大小写不敏感）
pub fn normalize_image_ext(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim().trim_start_matches('.');
    allowed_image_ext(trimmed)
}

/// Content-Type → 扩展名（`image/jpeg` / `image/jpg` 均映射到 `jpg`）
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

    #[test]
    fn validates_safe_image_names() {
        assert!(is_safe_image_name("1727000000_ab12cd.png"));
        assert!(is_safe_image_name("a.jpg"));
        assert!(!is_safe_image_name(""));
        assert!(!is_safe_image_name(".hidden.png"));
        assert!(!is_safe_image_name("../evil.png"));
        assert!(!is_safe_image_name("a/b.png"));
        assert!(!is_safe_image_name("a..b.png"));
        assert!(!is_safe_image_name(&"a".repeat(MAX_IMAGE_NAME_LEN + 1)));
    }

    #[test]
    fn maps_extensions_and_content_types() {
        assert_eq!(image_content_type("a.PNG"), "image/png");
        assert_eq!(image_content_type("a.jpeg"), "image/jpeg");
        assert_eq!(image_content_type("a.svg"), "application/octet-stream");
        assert_eq!(allowed_image_ext("JPG"), Some("jpg"));
        assert_eq!(allowed_image_ext(".jpg"), None);
        assert_eq!(normalize_image_ext(" .Jpeg "), Some("jpeg"));
        assert_eq!(normalize_image_ext("svg"), None);
        assert_eq!(image_ext_for_content_type("image/jpg"), Some("jpg"));
        assert_eq!(
            image_ext_for_content_type("image/webp; charset=binary"),
            Some("webp")
        );
    }
}
