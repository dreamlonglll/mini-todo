//! 收纳文件夹的创建 / 重命名 / 删除 / 列举。
//!
//! 只操作收纳根目录下我们自己的文件夹，用 `std::fs` 实现，全平台可用。
//! 对外函数使用默认收纳根目录；`*_at(root, ...)` 版本供单元测试在临时目录里调用。

use super::location::fence_root_dir;
use super::naming::{
    dedupe_name, display_name, is_auto_hidden_name, is_shell_metadata_file, is_shortcut_name,
    is_valid_folder_name, sanitize_folder_name,
};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 收纳文件夹里的一项
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FenceEntry {
    /// 收纳文件夹内的实际文件名（= `fence_items.file_name`）
    pub file_name: String,
    /// 显示名：`.lnk` / `.url` 去掉后缀，其他原样
    pub display_name: String,
    /// 完整路径
    pub path: String,
    /// 是否文件夹（指向文件夹的符号链接 / 联接点也算）
    pub is_dir: bool,
    /// 是否快捷方式（`.lnk` / `.url`）
    pub is_shortcut: bool,
    /// 修改时间（Unix 毫秒），取不到时为 `None`
    pub modified_ms: Option<i64>,
    /// 用户右键「隐藏」的（`fence_items.hidden`，对账时从行里带出；文件层列举时一律为 `false`）
    pub hidden: bool,
    /// 以 `.` 开头、按名字自动隐藏的（[`is_auto_hidden_name`]），不能单独取消隐藏
    pub auto_hidden: bool,
}

impl FenceEntry {
    /// 是否隐藏（用户隐藏或自动隐藏）：收纳箱没开「显示隐藏的图标」时不显示，开了时半透明显示
    pub fn is_hidden(&self) -> bool {
        self.hidden || self.auto_hidden
    }
}

/// 路径上是否已有东西（含失效的符号链接）
pub(crate) fn entry_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// 目录下已有项的名字（小写）。用于不区分大小写的重名判断：NTFS 默认不区分大小写，
/// 但个别目录可以开启区分大小写，其他平台的文件系统也区分，统一按名字列表比较才可靠。
/// 目录不存在或读取失败时返回空集合。
pub(crate) fn existing_names_lower(dir: &Path) -> HashSet<String> {
    fs::read_dir(dir)
        .map(|read| {
            read.flatten()
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// 是否带隐藏 / 系统属性（列举用：这类文件永不显示，"显示隐藏的图标"也不显示）：
/// Windows 看 `FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM`；macOS 看 `UF_HIDDEN`（`chflags hidden`，访达不显示）。
/// 其他平台没有这类属性，恒为 `false`。以 `.` 开头的项目不看这里，照常列出、标记为自动隐藏（[`FenceEntry::auto_hidden`]）
pub(crate) fn has_hidden_or_system_attribute(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
        meta.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0
    }
    #[cfg(target_os = "macos")]
    {
        super::mac::has_hidden_flag(meta)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = meta;
        false
    }
}

/// 是否是隐藏或系统文件（移入校验用，这类文件不收纳，避免"移走后哪里都看不到"）：
/// Windows 看文件属性、macOS 看 `UF_HIDDEN`（两者的点文件都照常移入，移入后按名字自动隐藏）；
/// 其他平台看是否以 `.` 开头
pub(crate) fn is_hidden_or_system(meta: &fs::Metadata, file_name: &str) -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let _ = file_name;
        has_hidden_or_system_attribute(meta)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = meta;
        is_auto_hidden_name(file_name)
    }
}

/// 是否是文件夹（符号链接 / 联接点按目标判断）
pub(crate) fn path_is_dir(path: &Path, meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
    } else {
        meta.is_dir()
    }
}

fn modified_ms(meta: &fs::Metadata) -> Option<i64> {
    let duration = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_millis()).ok()
}

/// 常见 IO 错误的中文说明
pub(crate) fn describe_io_error(err: &std::io::Error) -> String {
    // Win32 错误码（在其他平台上同样的数字是别的 errno，例如 macOS 的 32 = EPIPE）
    #[cfg(windows)]
    match err.raw_os_error() {
        // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
        Some(32) | Some(33) => return "正被其他程序占用".to_string(),
        // ERROR_DIR_NOT_EMPTY
        Some(145) => return "文件夹不为空".to_string(),
        _ => {}
    }
    match err.kind() {
        ErrorKind::NotFound => "不存在".to_string(),
        ErrorKind::PermissionDenied if cfg!(windows) => {
            "没有权限（可能正被其他程序占用）".to_string()
        }
        ErrorKind::PermissionDenied => "没有权限".to_string(),
        ErrorKind::AlreadyExists => "已存在同名项".to_string(),
        ErrorKind::DirectoryNotEmpty => "文件夹不为空".to_string(),
        _ => err.to_string(),
    }
}

/// 校验文件夹名并拼出完整路径（不访问文件系统）
pub(crate) fn folder_path_at(root: &Path, folder_name: &str) -> Result<PathBuf, String> {
    if !is_valid_folder_name(folder_name) {
        return Err(format!("收纳文件夹名不合法: {:?}", folder_name));
    }
    Ok(root.join(folder_name))
}

/// 收纳文件夹的完整路径（校验名字，不创建、不检查是否存在）
pub fn fence_folder_path(folder_name: &str) -> Result<PathBuf, String> {
    folder_path_at(&fence_root_dir()?, folder_name)
}

fn lowercase_set<'a>(names: impl IntoIterator<Item = &'a String>) -> HashSet<String> {
    names.into_iter().map(|n| n.to_lowercase()).collect()
}

pub(crate) fn create_folder_at(
    root: &Path,
    title: &str,
    taken: &[String],
) -> Result<String, String> {
    fs::create_dir_all(root)
        .map_err(|e| format!("创建收纳根目录失败: {}", describe_io_error(&e)))?;
    let base = sanitize_folder_name(title);
    let taken = lowercase_set(taken);
    let mut raced: HashSet<String> = HashSet::new();
    // 去重与创建之间可能有别人抢先建了同名目录，最多重试几次
    for _ in 0..16 {
        let on_disk = existing_names_lower(root);
        let name = dedupe_name(&base, |candidate| {
            let lower = candidate.to_lowercase();
            taken.contains(&lower) || raced.contains(&lower) || on_disk.contains(&lower)
        });
        match fs::create_dir(root.join(&name)) {
            Ok(()) => return Ok(name),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                raced.insert(name.to_lowercase());
            }
            Err(e) => {
                return Err(format!(
                    "创建收纳文件夹「{}」失败: {}",
                    name,
                    describe_io_error(&e)
                ))
            }
        }
    }
    Err("创建收纳文件夹失败：同名文件夹过多".to_string())
}

/// 新建收纳箱时创建对应文件夹，返回实际文件夹名（写入 `fences.folder_name`）。
///
/// * `title`：收纳箱标题，经 [`sanitize_folder_name`] 清洗
/// * `taken`：数据库里已有的全部 `folder_name`（即使对应文件夹已被用户删掉，也要避开，防止 UNIQUE 冲突）
///
/// 与磁盘上已有的同名项（不区分大小写）同样避开，重名加 ` (2)`、` (3)` 后缀。
pub fn create_fence_folder(title: &str, taken: &[String]) -> Result<String, String> {
    create_folder_at(&fence_root_dir()?, title, taken)
}

pub(crate) fn ensure_folder_at(root: &Path, folder_name: &str) -> Result<PathBuf, String> {
    let dir = folder_path_at(root, folder_name)?;
    fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "创建收纳文件夹「{}」失败: {}",
            folder_name,
            describe_io_error(&e)
        )
    })?;
    Ok(dir)
}

/// 确保收纳文件夹存在（不存在则按原名重建），返回完整路径。
/// 用于应用启动恢复收纳箱时：用户可能在资源管理器里删掉了文件夹。
pub fn ensure_fence_folder(folder_name: &str) -> Result<PathBuf, String> {
    ensure_folder_at(&fence_root_dir()?, folder_name)
}

pub(crate) fn rename_folder_at(
    root: &Path,
    old_folder: &str,
    new_title: &str,
    taken: &[String],
) -> Result<String, String> {
    let old_path = folder_path_at(root, old_folder)?;
    let base = sanitize_folder_name(new_title);
    if base == old_folder {
        return Ok(old_folder.to_string());
    }
    let old_lower = old_folder.to_lowercase();
    let others: Vec<String> = taken
        .iter()
        .filter(|t| t.to_lowercase() != old_lower)
        .cloned()
        .collect();
    if !entry_exists(&old_path) {
        // 旧文件夹已不在（用户手动删了），里面没有可丢的东西，直接按新标题建一个
        return create_folder_at(root, new_title, &others);
    }
    let others = lowercase_set(&others);
    let on_disk = existing_names_lower(root);
    let new_name = dedupe_name(&base, |candidate| {
        let lower = candidate.to_lowercase();
        // 旧文件夹自身不算占用：允许只改大小写，或去重后又落回原名
        lower != old_lower && (others.contains(&lower) || on_disk.contains(&lower))
    });
    if new_name == old_folder {
        return Ok(new_name);
    }
    fs::rename(&old_path, root.join(&new_name)).map_err(|e| {
        format!(
            "收纳文件夹「{}」改名为「{}」失败: {}",
            old_folder,
            new_name,
            describe_io_error(&e)
        )
    })?;
    Ok(new_name)
}

/// 收纳箱改标题时同步改文件夹名，返回新的文件夹名（与旧名相同表示无需改动）。
///
/// * 新名字按 [`sanitize_folder_name`] 清洗，与 `taken`（数据库里其他收纳箱的 `folder_name`，
///   可以包含旧名，内部会排除）及磁盘上已有项去重
/// * 只改大小写也会真正改名
/// * 旧文件夹不存在时直接按新标题新建文件夹
/// * 失败（文件夹被占用等）返回 `Err`，旧文件夹保持不动，调用方保留旧 `folder_name` 并提示
pub fn rename_fence_folder(
    old_folder: &str,
    new_title: &str,
    taken: &[String],
) -> Result<String, String> {
    rename_folder_at(&fence_root_dir()?, old_folder, new_title, taken)
}

/// 去掉只读属性后再试（Windows 上只读文件 / 文件夹无法直接删除）
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)] // Windows 上只清除 FILE_ATTRIBUTE_READONLY，不涉及 Unix 权限位
fn clear_readonly(path: &Path) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
    }
}

#[cfg(not(windows))]
fn clear_readonly(_path: &Path) {}

pub(crate) fn remove_folder_if_empty_at(root: &Path, folder_name: &str) -> Result<bool, String> {
    let dir = folder_path_at(root, folder_name)?;
    let meta = match fs::symlink_metadata(&dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(format!("读取收纳文件夹失败: {}", describe_io_error(&e))),
    };
    if !meta.is_dir() {
        return Err(format!("「{}」不是文件夹", folder_name));
    }
    let mut metadata_files = Vec::new();
    for entry in
        fs::read_dir(&dir).map_err(|e| format!("读取收纳文件夹失败: {}", describe_io_error(&e)))?
    {
        let entry = entry.map_err(|e| format!("读取收纳文件夹失败: {}", describe_io_error(&e)))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        if is_file && is_shell_metadata_file(&name) {
            metadata_files.push(entry.path());
        } else {
            return Ok(false);
        }
    }
    for file in metadata_files {
        clear_readonly(&file);
        fs::remove_file(&file)
            .map_err(|e| format!("删除 {} 失败: {}", file.display(), describe_io_error(&e)))?;
    }
    if let Err(first) = fs::remove_dir(&dir) {
        if first.kind() == ErrorKind::NotFound {
            return Ok(true);
        }
        clear_readonly(&dir);
        fs::remove_dir(&dir).map_err(|e| {
            format!(
                "删除收纳文件夹「{}」失败: {}",
                folder_name,
                describe_io_error(&e)
            )
        })?;
    }
    Ok(true)
}

/// 删除收纳文件夹，但只在它已经清空时删除（只剩 `desktop.ini` / `Thumbs.db` / `.DS_Store` 等系统元数据文件也算空，会一并删掉）。
///
/// 返回 `Ok(true)` 表示已删除或本来就不存在；`Ok(false)` 表示里面还有文件，文件夹保留。
/// 绝不递归删除用户文件。
pub fn remove_fence_folder_if_empty(folder_name: &str) -> Result<bool, String> {
    remove_folder_if_empty_at(&fence_root_dir()?, folder_name)
}

pub(crate) fn list_items_at(root: &Path, folder_name: &str) -> Result<Vec<FenceEntry>, String> {
    let dir = folder_path_at(root, folder_name)?;
    let read = fs::read_dir(&dir).map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            format!("收纳文件夹「{}」不存在", folder_name)
        } else {
            format!("读取收纳文件夹失败: {}", describe_io_error(&e))
        }
    })?;
    let mut items: Vec<FenceEntry> = read
        .flatten()
        .filter_map(|entry| {
            // 含非法 UTF-16 的名字无法在前后端间传递，跳过
            let file_name = entry.file_name().into_string().ok()?;
            if is_shell_metadata_file(&file_name) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if has_hidden_or_system_attribute(&meta) {
                return None;
            }
            let path = entry.path();
            let is_dir = path_is_dir(&path, &meta);
            Some(FenceEntry {
                display_name: display_name(&file_name, is_dir),
                is_shortcut: !is_dir && is_shortcut_name(&file_name),
                path: path.to_string_lossy().into_owned(),
                modified_ms: modified_ms(&meta),
                hidden: false,
                auto_hidden: is_auto_hidden_name(&file_name),
                file_name,
                is_dir,
            })
        })
        .collect();
    items.sort_by_key(|item| item.file_name.to_lowercase());
    Ok(items)
}

/// 列举收纳文件夹内容（按文件名不区分大小写排序；跳过 `desktop.ini`、`Thumbs.db`、`.DS_Store` 等系统元数据文件，
/// 与带隐藏 / 系统属性的文件：Windows 的隐藏 / 系统属性、macOS 的 `UF_HIDDEN`）。
/// 以 `.` 开头的项目照常列出、`auto_hidden = true`；`hidden`（用户隐藏）由对账从数据库带出，这里一律 `false`。
/// 文件夹不存在返回 `Err`。
pub fn list_fence_items(folder_name: &str) -> Result<Vec<FenceEntry>, String> {
    list_items_at(&fence_root_dir()?, folder_name)
}

pub(crate) fn list_folders_at(root: &Path) -> Result<Vec<String>, String> {
    let read = match fs::read_dir(root) {
        Ok(read) => read,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取收纳根目录失败: {}", describe_io_error(&e))),
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_valid_folder_name(name))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    Ok(names)
}

/// 收纳根目录下现有的子文件夹名（启动对账 / 诊断用；不含名字不合法的目录）
pub fn list_fence_folders() -> Result<Vec<String>, String> {
    list_folders_at(&fence_root_dir()?)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 测试用临时目录：位于系统临时目录下，名字唯一，drop 时递归删除
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "mini-todo-fence-test-{}-{}-{}",
                tag,
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"x").unwrap();
    }
}

/// macOS 测试辅助：文件标志（`chflags`）
#[cfg(all(test, target_os = "macos"))]
pub(crate) mod mac_test_support {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    fn chflags(path: &Path, flags: u32) {
        let c = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: 有效的 NUL 结尾路径
        let rc = unsafe { libc::chflags(c.as_ptr(), flags) };
        assert_eq!(
            rc,
            0,
            "chflags {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }

    /// `chflags hidden`
    pub fn set_hidden_flag(path: &Path) {
        chflags(path, libc::UF_HIDDEN);
    }

    /// `chflags uchg`（访达「已锁定」）
    pub fn lock(path: &Path) {
        chflags(path, libc::UF_IMMUTABLE);
    }

    /// 清除全部用户标志（解锁，测试结束前调用，否则临时目录删不掉）
    pub fn clear_flags(path: &Path) {
        chflags(path, 0);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::mac_test_support;
    use super::test_support::{touch, TempDir};
    use super::*;

    #[test]
    fn create_dedupes_against_disk_and_taken() {
        let tmp = TempDir::new("create");
        let root = tmp.path().join("root");
        assert_eq!(create_folder_at(&root, "收纳箱", &[]).unwrap(), "收纳箱");
        assert!(root.join("收纳箱").is_dir());
        // 磁盘上已有
        assert_eq!(
            create_folder_at(&root, "收纳箱", &[]).unwrap(),
            "收纳箱 (2)"
        );
        // 数据库里有但磁盘上没有，也要避开
        let taken = vec!["收纳箱 (3)".to_string()];
        assert_eq!(
            create_folder_at(&root, "收纳箱", &taken).unwrap(),
            "收纳箱 (4)"
        );
        // 不区分大小写；清洗非法字符
        assert_eq!(create_folder_at(&root, "Tools", &[]).unwrap(), "Tools");
        assert_eq!(create_folder_at(&root, "tools", &[]).unwrap(), "tools (2)");
        assert_eq!(create_folder_at(&root, "a:b", &[]).unwrap(), "a_b");
        // 与根目录下的同名文件冲突
        touch(&root.join("file"));
        assert_eq!(create_folder_at(&root, "file", &[]).unwrap(), "file (2)");
    }

    #[test]
    fn rename_folder_cases() {
        let tmp = TempDir::new("rename");
        let root = tmp.path();
        let name = create_folder_at(root, "旧标题", &[]).unwrap();
        touch(&root.join(&name).join("a.lnk"));

        // 名字不变
        assert_eq!(
            rename_folder_at(root, &name, " 旧标题 ", &[]).unwrap(),
            "旧标题"
        );
        // 正常改名，内容随之移动
        let renamed = rename_folder_at(root, &name, "新标题", std::slice::from_ref(&name)).unwrap();
        assert_eq!(renamed, "新标题");
        assert!(root.join("新标题").join("a.lnk").is_file());
        assert!(!root.join("旧标题").exists());

        // 与其他收纳箱重名：加后缀
        create_folder_at(root, "常用", &[]).unwrap();
        let renamed = rename_folder_at(root, "新标题", "常用", &["常用".to_string()]).unwrap();
        assert_eq!(renamed, "常用 (2)");

        // 去重后落回自身：不动
        let same = rename_folder_at(root, "常用 (2)", "常用", &["常用".to_string()]).unwrap();
        assert_eq!(same, "常用 (2)");

        // 只改大小写
        create_folder_at(root, "abc", &[]).unwrap();
        assert_eq!(rename_folder_at(root, "abc", "ABC", &[]).unwrap(), "ABC");
        let names = list_folders_at(root).unwrap();
        assert!(names.contains(&"ABC".to_string()), "{names:?}");

        // 旧文件夹不存在：按新标题新建
        assert_eq!(
            rename_folder_at(root, "不存在", "重建", &[]).unwrap(),
            "重建"
        );
        assert!(root.join("重建").is_dir());

        // 非法旧名
        assert!(rename_folder_at(root, "..", "x", &[]).is_err());
    }

    #[test]
    fn remove_only_when_empty() {
        let tmp = TempDir::new("remove");
        let root = tmp.path();
        // 不存在算已删除
        assert!(remove_folder_if_empty_at(root, "没有").unwrap());

        let name = create_folder_at(root, "箱", &[]).unwrap();
        touch(&root.join(&name).join("keep.txt"));
        assert!(!remove_folder_if_empty_at(root, &name).unwrap());
        assert!(root.join(&name).join("keep.txt").is_file());

        fs::remove_file(root.join(&name).join("keep.txt")).unwrap();
        touch(&root.join(&name).join("desktop.ini"));
        touch(&root.join(&name).join("Thumbs.db"));
        // 访达写的元数据同样不算内容（访达打开过收纳文件夹后必有 .DS_Store）
        touch(&root.join(&name).join(".DS_Store"));
        touch(&root.join(&name).join(".localized"));
        #[cfg(not(windows))] // Windows 文件名不能含控制字符
        touch(&root.join(&name).join("Icon\r"));
        assert!(remove_folder_if_empty_at(root, &name).unwrap());
        assert!(!root.join(&name).exists());

        // 只有元数据文件名的文件夹不算元数据
        let name = create_folder_at(root, "箱3", &[]).unwrap();
        fs::create_dir(root.join(&name).join(".DS_Store")).unwrap();
        assert!(!remove_folder_if_empty_at(root, &name).unwrap());

        // 子文件夹也算内容
        let name = create_folder_at(root, "箱2", &[]).unwrap();
        fs::create_dir(root.join(&name).join("sub")).unwrap();
        assert!(!remove_folder_if_empty_at(root, &name).unwrap());

        assert!(remove_folder_if_empty_at(root, "../x").is_err());
    }

    #[test]
    fn list_items_filters_and_describes() {
        let tmp = TempDir::new("list");
        let root = tmp.path();
        let name = create_folder_at(root, "箱", &[]).unwrap();
        let dir = root.join(&name);
        touch(&dir.join("Chrome.lnk"));
        touch(&dir.join("官网.url"));
        touch(&dir.join("b.txt"));
        touch(&dir.join("desktop.ini"));
        touch(&dir.join(".DS_Store"));
        touch(&dir.join(".localized"));
        #[cfg(not(windows))] // Windows 文件名不能含控制字符
        touch(&dir.join("Icon\r"));
        fs::create_dir(dir.join("文件夹.lnk")).unwrap();
        // 以 . 开头的项目（没有隐藏属性）所有平台都列出，标记为自动隐藏
        touch(&dir.join(".gitkeep"));
        fs::create_dir(dir.join(".config")).unwrap();
        #[cfg(windows)]
        {
            // 带隐藏属性的文件永不列出
            touch(&dir.join("hidden.txt"));
            let status = std::process::Command::new("attrib")
                .arg("+h")
                .arg(dir.join("hidden.txt"))
                .status()
                .unwrap();
            assert!(status.success());
        }
        #[cfg(target_os = "macos")]
        {
            // 带 UF_HIDDEN（chflags hidden）的文件永不列出，同 Windows 的隐藏属性
            touch(&dir.join("hidden.txt"));
            mac_test_support::set_hidden_flag(&dir.join("hidden.txt"));
        }

        let items = list_items_at(root, &name).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.file_name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                ".config",
                ".gitkeep",
                "b.txt",
                "Chrome.lnk",
                "官网.url",
                "文件夹.lnk"
            ]
        );
        assert!(items[0].auto_hidden && items[0].is_dir && items[0].is_hidden());
        assert!(items[1].auto_hidden && !items[1].is_dir);
        assert!(
            items.iter().all(|i| !i.hidden),
            "用户隐藏标记来自数据库，文件层列举一律为 false"
        );

        let chrome = &items[3];
        assert_eq!(chrome.display_name, "Chrome");
        assert!(chrome.is_shortcut && !chrome.is_dir);
        assert!(!chrome.auto_hidden && !chrome.is_hidden());
        assert_eq!(PathBuf::from(&chrome.path), dir.join("Chrome.lnk"));
        assert!(chrome.modified_ms.is_some());

        assert_eq!(items[4].display_name, "官网");
        let folder = &items[5];
        assert!(folder.is_dir && !folder.is_shortcut);
        assert_eq!(folder.display_name, "文件夹.lnk");

        assert!(list_items_at(root, "不存在")
            .unwrap_err()
            .contains("不存在"));
        assert!(list_items_at(root, "a/b").is_err());
    }

    #[test]
    fn list_folders_skips_files_and_missing_root() {
        let tmp = TempDir::new("folders");
        let root = tmp.path();
        assert!(list_folders_at(&root.join("missing")).unwrap().is_empty());
        create_folder_at(root, "b", &[]).unwrap();
        create_folder_at(root, "A", &[]).unwrap();
        touch(&root.join("file.txt"));
        assert_eq!(list_folders_at(root).unwrap(), vec!["A", "b"]);
    }
}
