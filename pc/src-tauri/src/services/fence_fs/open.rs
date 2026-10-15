//! 打开项目与在资源管理器中定位。

use std::path::Path;

/// 打开收纳箱里的项目（双击）：`ShellExecuteExW` 默认动作。
///
/// * exe / bat / cmd 把工作目录设为其所在目录；`.lnk` 按快捷方式自带的参数与工作目录启动；
///   `.url` 用默认浏览器打开；文件夹在资源管理器中打开；普通文件用关联程序打开
/// * `SEE_MASK_FLAG_NO_UI`：目标失效的快捷方式不弹系统对话框，返回
///   `Err("快捷方式指向的目标不存在，可能已被移动或卸载")`；需要管理员权限的程序仍会弹 UAC，
///   用户取消返回 `Err("已取消")`
/// * 文件不存在返回 `Err("文件不存在")`；非 Windows 返回 `Err`
/// * 阻塞到 Shell 完成启动（新开 STA 线程执行），请在 `spawn_blocking` 中调用
pub fn launch_item(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if std::fs::symlink_metadata(path).is_err() {
        return Err("文件不存在".to_string());
    }
    #[cfg(windows)]
    {
        let path = path.to_path_buf();
        super::sta::run_in_new_sta("fence-launch", move || super::shell::launch(&path))
    }
    #[cfg(not(windows))]
    {
        Err(super::UNSUPPORTED.to_string())
    }
}

/// 在资源管理器中打开所在文件夹并选中该项（右键「打开所在位置」）。
///
/// 首选 `SHOpenFolderAndSelectItems`（复用已打开的资源管理器窗口、无需处理引号转义），
/// 失败时退回 `explorer.exe /select,"<路径>"`。路径不存在返回 `Err`。
pub fn reveal_in_explorer(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if std::fs::symlink_metadata(path).is_err() {
        return Err("文件不存在".to_string());
    }
    #[cfg(windows)]
    {
        let target = path.to_path_buf();
        let primary =
            super::sta::run_in_new_sta("fence-reveal", move || super::shell::reveal(&target));
        match primary {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!(
                    "[fence] SHOpenFolderAndSelectItems 失败，改用 explorer /select: {}",
                    e
                );
                use std::os::windows::process::CommandExt;
                std::process::Command::new("explorer.exe")
                    .raw_arg(format!("/select,\"{}\"", path.display()))
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| format!("打开所在位置失败: {}", e))
            }
        }
    }
    #[cfg(not(windows))]
    {
        Err(super::UNSUPPORTED.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_paths_are_rejected_without_launching() {
        let missing = std::env::temp_dir().join("mini-todo-fence-definitely-missing.lnk");
        let missing = missing.to_string_lossy();
        assert_eq!(launch_item(&missing).unwrap_err(), "文件不存在");
        assert_eq!(reveal_in_explorer(&missing).unwrap_err(), "文件不存在");
    }
}
