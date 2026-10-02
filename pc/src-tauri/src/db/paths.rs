//! 应用数据目录与文件命名规则。
//!
//! 数据目录：`dirs::data_local_dir()/mini-todo`（Windows 上是 `%LOCALAPPDATA%\mini-todo`），
//! 下有 `data.db`、`images/`、`backups/`。以前 todo.rs / sync_cmd.rs 各自拼一份路径，
//! 现在统一从这里取。

use std::path::PathBuf;

/// 应用数据根目录
pub fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mini-todo")
}

/// SQLite 数据库文件
pub fn db_path() -> PathBuf {
    app_data_dir().join("data.db")
}

/// 用户上传的图片目录（子任务 / 描述里的 Markdown 图片）
pub fn images_dir() -> PathBuf {
    app_data_dir().join("images")
}

/// 图片文件名最大长度（K5）
const MAX_IMAGE_NAME_LEN: usize = 128;

/// K5 安全图片文件名：单个普通路径段，匹配 `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$` 且不含 `..`。
///
/// 保存、上传、下载、列举图片时一律先过这一关：名字来自远端 JSON / PROPFIND / 前端，
/// 不校验就 `images_dir.join(name)` 会被 `../` 或绝对路径带出图片目录（B2）。
/// 不满足规则的名字由调用方跳过并记日志。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_generated_image_names() {
        for ok in [
            "1715000000000_ab12cd.png",
            "img_1715000000000_42.jpeg",
            "a",
            "A.B-c_d.webp",
            "0.gif",
        ] {
            assert!(is_safe_image_name(ok), "应接受 {ok:?}");
        }
        assert!(is_safe_image_name(&"a".repeat(128)));
    }

    #[test]
    fn rejects_traversal_and_odd_names() {
        for bad in [
            "",
            "..",
            "../evil.png",
            "a/../../b.png",
            "a..png",
            "/etc/passwd",
            "C:\\Windows\\x.png",
            "dir/a.png",
            "dir\\a.png",
            ".hidden.png",
            "-flag.png",
            "_x.png",
            "a b.png",
            "截图.png",
            "a.png\0",
            "a%2F..%2Fb.png",
        ] {
            assert!(!is_safe_image_name(bad), "应拒绝 {bad:?}");
        }
        assert!(!is_safe_image_name(&"a".repeat(129)));
    }
}
