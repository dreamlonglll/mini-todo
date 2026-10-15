//! 应用数据目录与文件命名规则。
//!
//! 数据目录：`dirs::data_local_dir()/mini-todo`（Windows 上是 `%LOCALAPPDATA%\mini-todo`），
//! 下有 `data.db`、`images/`、`backups/`。以前 todo.rs / sync_cmd.rs 各自拼一份路径，
//! 现在统一从这里取。
//!
//! 开发专用数据隔离：debug 构建下设置了 [`DEV_DATA_DIR_ENV`] 时，数据根目录改为该目录
//! （见 [`dev_data_dir_override`]）；release 构建完全忽略。

use std::ffi::OsString;
use std::path::PathBuf;

/// 开发专用：覆盖应用数据根目录的环境变量，**仅 debug 构建生效**。
///
/// 用户日常运行着安装版，安装版启用了单实例、默认共用真实库与 WebDAV。开发版设置它之后：
/// - 库、图片、备份、节假日缓存都落到该目录（全部经 [`app_data_dir`] 取路径）；新库没有 WebDAV
///   配置，因此不会同步
/// - 日志也改到该目录下（app 的 `services::logging`），不与安装版共用日志文件
/// - 不启用单实例（app 的 `services::single_instance`）、不刷新开机自启注册表项（app 的 `lib.rs`），
///   两个 `mini-todo.exe` 可以并存
///
/// release 构建完全忽略它：[`dev_data_dir_override`] 在 release 下根本不读环境变量，
/// 纯函数 [`data_dir_override`] 在 `debug_build == false` 时恒为 `None`（有单元测试）。
pub const DEV_DATA_DIR_ENV: &str = "MINI_TODO_DATA_DIR";

/// 开发专用数据隔离目录：debug 构建且设置了非空的 [`DEV_DATA_DIR_ENV`] 时返回该目录（绝对路径），
/// 其它情况（含所有 release 构建）返回 `None`。
pub fn dev_data_dir_override() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        data_dir_override(std::env::var_os(DEV_DATA_DIR_ENV), true)
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// 按构建类型解析覆盖目录（纯函数，便于测试 release 语义）：
/// release（`debug_build == false`）一律 `None`；空值视为未设置；相对路径按当前目录转为绝对路径。
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn data_dir_override(raw: Option<OsString>, debug_build: bool) -> Option<PathBuf> {
    if !debug_build {
        return None;
    }
    let path = PathBuf::from(raw.filter(|value| !value.is_empty())?);
    Some(std::path::absolute(&path).unwrap_or(path))
}

/// 应用数据根目录（debug 构建可被 [`DEV_DATA_DIR_ENV`] 覆盖）
pub fn app_data_dir() -> PathBuf {
    if let Some(dir) = dev_data_dir_override() {
        return dir;
    }
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
    fn release_builds_ignore_the_dev_data_dir_variable() {
        assert_eq!(data_dir_override(Some("D:\\dev-data".into()), false), None);
        assert_eq!(data_dir_override(None, false), None);
    }

    #[test]
    fn debug_builds_use_a_non_empty_dev_data_dir() {
        assert_eq!(data_dir_override(None, true), None, "未设置");
        assert_eq!(
            data_dir_override(Some("".into()), true),
            None,
            "空值视为未设置"
        );

        let absolute = std::env::temp_dir().join("mini-todo-dev-data");
        assert_eq!(
            data_dir_override(Some(absolute.clone().into_os_string()), true),
            Some(absolute)
        );

        let relative = data_dir_override(Some("dev-data".into()), true).unwrap();
        assert!(relative.is_absolute(), "相对路径转为绝对路径：{relative:?}");
        assert!(relative.ends_with("dev-data"));
    }

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
