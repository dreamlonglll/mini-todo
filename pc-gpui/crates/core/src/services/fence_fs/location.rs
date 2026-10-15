//! 路径解析与判定：收纳根目录、图标缓存目录、用户桌面 / 公共桌面、OneDrive 重定向。

use std::path::{Path, PathBuf};

/// 收纳根目录在用户目录下的名字：`%USERPROFILE%\MiniTodo收纳箱\`
pub const FENCE_ROOT_DIR_NAME: &str = "MiniTodo收纳箱";

/// 开发专用：覆盖收纳根目录的环境变量（仅 debug 构建生效，见 [`fence_root_override`]）
// release 构建不读它（fence_root_override 的 release 分支恒为 None）
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub const FENCES_ROOT_ENV: &str = "MINI_TODO_FENCES_ROOT";

/// 图标缓存目录名（位于应用数据目录下）
const ICON_CACHE_DIR_NAME: &str = "fence-icons";

/// 开发专用覆盖：debug 构建下读取 `MINI_TODO_FENCES_ROOT`，用于实测时把收纳文件夹隔离到临时目录。
///
/// **release 构建完全忽略该变量**（`cfg(not(debug_assertions))` 分支恒为 `None`），
/// 安装版用户即使设置了同名环境变量，收纳根目录也始终是 `%USERPROFILE%\MiniTodo收纳箱`。
fn fence_root_override() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        root_override_from(std::env::var_os(FENCES_ROOT_ENV))
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// 环境变量值 → 覆盖目录：未设置或空白时不覆盖
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn root_override_from(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let value = value?;
    if value.to_string_lossy().trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

/// 收纳根目录的路径（不创建）。
///
/// 默认 `%USERPROFILE%\MiniTodo收纳箱`；debug 构建设置了 `MINI_TODO_FENCES_ROOT` 时用该目录。
pub fn fence_root_path() -> Result<PathBuf, String> {
    if let Some(dir) = fence_root_override() {
        return Ok(dir);
    }
    dirs::home_dir()
        .map(|home| home.join(FENCE_ROOT_DIR_NAME))
        .ok_or_else(|| "无法确定用户目录，不能创建收纳文件夹".to_string())
}

/// 收纳根目录（不存在则创建）
pub fn fence_root_dir() -> Result<PathBuf, String> {
    let root = fence_root_path()?;
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("创建收纳根目录失败（{}）: {}", root.display(), e))?;
    Ok(root)
}

/// 图标缓存目录 `<应用数据目录>\fence-icons`（Windows 上默认 `%LOCALAPPDATA%\mini-todo\fence-icons`）。
///
/// 跟随 `db::paths::app_data_dir()`，开发版的数据目录隔离对它同样生效。不负责创建。
pub fn icon_cache_dir() -> PathBuf {
    crate::db::paths::app_data_dir().join(ICON_CACHE_DIR_NAME)
}

/// 用户桌面（`dirs::desktop_dir()`，底层是 `SHGetKnownFolderPath(FOLDERID_Desktop)`，已处理 OneDrive / 组策略重定向）
pub fn user_desktop_dir() -> Option<PathBuf> {
    dirs::desktop_dir()
}

/// 公共桌面（通常是 `C:\Users\Public\Desktop`）。非 Windows 返回 `None`。
pub fn public_desktop_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        super::shell::known_folder_public_desktop().or_else(|| {
            std::env::var_os("PUBLIC").map(|public| PathBuf::from(public).join("Desktop"))
        })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 用户桌面是否被 OneDrive 重定向（移出桌面的文件会同步影响其他设备，首次收纳时提示一次）
pub fn desktop_is_onedrive_redirected() -> bool {
    match user_desktop_dir() {
        Some(desktop) => is_under_onedrive(&desktop, &onedrive_roots()),
        None => false,
    }
}

/// OneDrive 客户端写入的根目录环境变量（个人版 / 商业版）
fn onedrive_roots() -> Vec<PathBuf> {
    ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// 路径是否在 OneDrive 下：位于任一 OneDrive 根目录内，或者某一级目录名以 `OneDrive` 开头
/// （覆盖 `OneDrive - 公司名` 且环境变量缺失的情况）。
pub fn is_under_onedrive(path: &Path, onedrive_roots: &[PathBuf]) -> bool {
    if onedrive_roots
        .iter()
        .any(|root| path_starts_with_ci(path, root))
    {
        return true;
    }
    normalize_for_compare(path)
        .split('/')
        .any(|segment| segment.starts_with("onedrive"))
}

/// 用于比较的规范形式：去掉 `\\?\` 前缀、统一成 `/`、去掉末尾分隔符、转小写
pub(crate) fn normalize_for_compare(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let raw = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.into_owned()
    };
    let unified = raw.replace('\\', "/");
    let trimmed = if unified.len() > 1 {
        unified.trim_end_matches('/')
    } else {
        unified.as_str()
    };
    trimmed.to_lowercase()
}

/// `path` 是否等于 `prefix` 或位于其下（按路径段、不区分大小写、忽略 `\\?\` 前缀）
pub fn path_starts_with_ci(path: &Path, prefix: &Path) -> bool {
    let path = normalize_for_compare(path);
    let prefix = normalize_for_compare(prefix);
    if prefix.is_empty() {
        return false;
    }
    path == prefix
        || (path.starts_with(&prefix)
            && (prefix.ends_with('/') || path[prefix.len()..].starts_with('/')))
}

/// 两个路径是否指向同一位置（不区分大小写、忽略 `\\?\` 前缀与末尾分隔符）
pub fn same_path_ci(a: &Path, b: &Path) -> bool {
    normalize_for_compare(a) == normalize_for_compare(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_override_requires_non_blank_value() {
        assert_eq!(root_override_from(None), None);
        assert_eq!(root_override_from(Some("".into())), None);
        assert_eq!(root_override_from(Some("   ".into())), None);
        assert_eq!(
            root_override_from(Some(r"C:\tmp\fences".into())),
            Some(PathBuf::from(r"C:\tmp\fences"))
        );
    }

    #[test]
    fn default_root_is_under_home() {
        // 测试进程里没有设置覆盖变量时，根目录是 <home>/MiniTodo收纳箱；
        // release 构建下 fence_root_override 恒为 None（见函数注释），这里只校验默认分支的拼接
        if std::env::var_os(FENCES_ROOT_ENV).is_none() {
            let root = fence_root_path().unwrap();
            assert_eq!(root.file_name().unwrap(), FENCE_ROOT_DIR_NAME);
            assert_eq!(root.parent(), dirs::home_dir().as_deref());
        }
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn release_build_ignores_override() {
        assert_eq!(fence_root_override(), None);
    }

    #[test]
    fn prefix_matching_is_segment_aware_and_case_insensitive() {
        let public = Path::new(r"C:\Users\Public\Desktop");
        assert!(path_starts_with_ci(
            Path::new(r"C:\Users\Public\Desktop\Chrome.lnk"),
            public
        ));
        assert!(path_starts_with_ci(
            Path::new(r"c:\users\public\desktop\sub\a.txt"),
            public
        ));
        assert!(path_starts_with_ci(
            Path::new(r"\\?\C:\Users\Public\Desktop\a.lnk"),
            public
        ));
        assert!(path_starts_with_ci(
            Path::new(r"C:\Users\Public\Desktop"),
            Path::new(r"C:\Users\Public\Desktop\")
        ));
        // 只是名字前缀相同，不是子路径
        assert!(!path_starts_with_ci(
            Path::new(r"C:\Users\Public\Desktop2\a.lnk"),
            public
        ));
        assert!(!path_starts_with_ci(
            Path::new(r"C:\Users\me\Desktop\a.lnk"),
            public
        ));
        assert!(!path_starts_with_ci(Path::new(r"C:\x"), Path::new("")));
        assert!(path_starts_with_ci(
            Path::new(r"\\?\UNC\server\share\a"),
            Path::new(r"\\server\share")
        ));
    }

    #[test]
    fn same_path_comparison() {
        assert!(same_path_ci(
            Path::new(r"C:\Users\Me\Desktop\"),
            Path::new(r"c:\users\me\desktop")
        ));
        assert!(!same_path_ci(
            Path::new(r"C:\Users\Me\Desktop\a"),
            Path::new(r"C:\Users\Me\Desktop")
        ));
    }

    #[test]
    fn onedrive_detection() {
        let roots = vec![PathBuf::from(r"C:\Users\me\OneDrive")];
        assert!(is_under_onedrive(
            Path::new(r"C:\Users\me\OneDrive\桌面"),
            &roots
        ));
        assert!(is_under_onedrive(
            Path::new(r"C:\Users\me\onedrive\Desktop"),
            &roots
        ));
        assert!(!is_under_onedrive(
            Path::new(r"C:\Users\me\Desktop"),
            &roots
        ));
        // 环境变量缺失时按目录名识别企业版
        assert!(is_under_onedrive(
            Path::new(r"D:\OneDrive - Contoso\Desktop"),
            &[]
        ));
        assert!(!is_under_onedrive(Path::new(r"D:\Data\Desktop"), &[]));
        // 自定义 OneDrive 根目录名
        let roots = vec![PathBuf::from(r"E:\Cloud")];
        assert!(is_under_onedrive(Path::new(r"E:\Cloud\Desktop"), &roots));
    }
}
