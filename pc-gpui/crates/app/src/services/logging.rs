//! 日志：与旧版 tauri-plugin-log 相同的文件位置与轮转规则。
//!
//! - 文件：`%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`（Linux：`dirs::data_local_dir()` 下同名目录）；
//!   开发专用数据隔离（`MINI_TODO_DATA_DIR`，仅 debug）时为 `<dir>/logs/mini-todo.log`
//! - 超过 2MB 轮转：当前文件改名为 `mini-todo_<YYYY-MM-DD_HH-MM-SS>.log`，只保留 1 份旧日志
//! - 本地时间；格式 `[YYYY-MM-DD][HH:MM:SS][target][LEVEL] message`
//! - 级别：本应用与 core 为 Info，其它 crate（GPUI / wgpu 等）为 Warn；同时输出到标准输出
//! - 已知无害的第三方错误降为 Debug（不落盘）：GPUI 的拖放图像助手 `IDropTargetHelper` 在拖放源的数据对象没有拖动图像时
//!   每次 DragEnter / DragOver / Drop 都报 `DV_E_FORMATETC`（"Invalid FORMATETC structure (0x80040064)"），不影响拖入
//!
//! release 版是 Windows 子系统程序（没有控制台），日志必须落盘；业务代码统一用 `log::{info,warn,error}!`。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// 旧版 Tauri identifier（日志目录名沿用）
pub const LOG_IDENTIFIER: &str = "com.tauri-app.mini-todo";
pub const LOG_FILE_NAME: &str = "mini-todo.log";
const LOG_STEM: &str = "mini-todo";
/// 单个日志文件上限（与旧版 `max_file_size(2 * 1024 * 1024)` 一致）
pub const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
/// 保留的旧日志份数（旧版 `RotationStrategy::KeepSome(1)`）
pub const KEEP_ROTATED: usize = 1;

/// 日志目录
pub fn log_dir() -> PathBuf {
    if let Some(dir) = mini_todo_core::db::paths::dev_data_dir_override() {
        return dir.join("logs");
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(LOG_IDENTIFIER)
        .join("logs")
}

struct FileState {
    dir: PathBuf,
    file: Option<File>,
    size: u64,
}

struct AppLogger {
    state: Mutex<FileState>,
}

/// 本应用与 core 的日志 Info 起，其它 crate Warn 起
fn level_for(target: &str) -> Level {
    if target.starts_with("mini_todo") {
        Level::Info
    } else {
        Level::Warn
    }
}

/// `DV_E_FORMATETC`：GPUI（gpui-pre-windows `window.rs`）的 `drop_target_helper.DragEnter / DragOver / Drop(..).log_err()`
/// 在数据对象没有拖动图像格式（.NET `DataObject` 等）时每次都返回它；只是拖动图像画不出来
const DROP_HELPER_NOISE: &str = "0x80040064";

/// 实际记录的级别：GPUI 拖放图像助手的已知无害错误降为 Debug，其它不变。
/// GPUI 的 `log_err` 按调用处文件推 target（注册表路径里没有 `crates/` 时为空串），所以按源文件判断
fn effective_level(record: &Record) -> Level {
    if record.level() == Level::Error
        && !record.target().starts_with("mini_todo")
        && record.file().is_some_and(|f| f.contains("gpui"))
        && record.args().to_string().contains(DROP_HELPER_NOISE)
    {
        Level::Debug
    } else {
        record.level()
    }
}

impl Log for AppLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= level_for(metadata.target())
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) || effective_level(record) > level_for(record.target())
        {
            return;
        }
        let now = chrono::Local::now();
        let line = format!(
            "{}[{}][{}] {}\n",
            now.format("[%Y-%m-%d][%H:%M:%S]"),
            record.target(),
            record.level(),
            record.args()
        );
        let _ = std::io::stdout().lock().write_all(line.as_bytes());

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.file.is_some() && state.size + line.len() as u64 > MAX_FILE_SIZE {
            state.file = None;
            rotate(&state.dir, &now.format("%Y-%m-%d_%H-%M-%S").to_string());
            state.size = 0;
        }
        if state.file.is_none() {
            match open_log_file(&state.dir) {
                Ok((file, size)) => {
                    state.file = Some(file);
                    state.size = size;
                }
                Err(_) => return,
            }
        }
        if let Some(file) = state.file.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            state.size += line.len() as u64;
        }
    }

    fn flush(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = state.file.as_mut() {
            let _ = file.flush();
        }
    }
}

fn open_log_file(dir: &Path) -> std::io::Result<(File, u64)> {
    std::fs::create_dir_all(dir)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(LOG_FILE_NAME))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok((file, size))
}

/// 当前文件改名为带时间戳的旧日志，只保留最新 [`KEEP_ROTATED`] 份
fn rotate(dir: &Path, stamp: &str) {
    let current = dir.join(LOG_FILE_NAME);
    let mut target = dir.join(format!("{LOG_STEM}_{stamp}.log"));
    let mut n = 1;
    while target.exists() {
        target = dir.join(format!("{LOG_STEM}_{stamp}.{n}.log"));
        n += 1;
    }
    let _ = std::fs::rename(&current, &target);
    prune_rotated(dir, KEEP_ROTATED);
}

/// 删除多余的旧日志（文件名里的时间戳可按字典序比较新旧）
fn prune_rotated(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut rotated: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{LOG_STEM}_")) && n.ends_with(".log"))
        })
        .collect();
    rotated.sort();
    let excess = rotated.len().saturating_sub(keep);
    for path in rotated.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// 安装全局 logger（进程启动后尽早调用一次；重复调用无效果）
pub fn init() {
    let logger = AppLogger {
        state: Mutex::new(FileState {
            dir: log_dir(),
            file: None,
            size: 0,
        }),
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_one_old_file() {
        let dir = std::env::temp_dir().join(format!("mini-todo-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for stamp in [
            "2026-10-01_00-00-00",
            "2026-10-02_00-00-00",
            "2026-10-03_00-00-00",
        ] {
            std::fs::write(dir.join(LOG_FILE_NAME), stamp).unwrap();
            rotate(&dir, stamp);
        }
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["mini-todo_2026-10-03_00-00-00.log".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gpui_drop_helper_formatetc_errors_are_demoted() {
        let file = r"C:\Users\me\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\gpui-pre-windows-0.3.9\src\window.rs";
        let noise = format_args!("Invalid FORMATETC structure (0x80040064)");
        let record = Record::builder()
            .target("")
            .level(Level::Error)
            .file(Some(file))
            .args(noise)
            .build();
        assert_eq!(effective_level(&record), Level::Debug);
        assert!(effective_level(&record) > level_for(record.target()));

        // 同一处的其它错误、别的 crate 的同样文字、本应用自己的错误都不降级
        let other = format_args!("unable to set window content size");
        let record = Record::builder()
            .target("")
            .level(Level::Error)
            .file(Some(file))
            .args(other)
            .build();
        assert_eq!(effective_level(&record), Level::Error);
        let elsewhere = format_args!("Invalid FORMATETC structure (0x80040064)");
        let record = Record::builder()
            .target("rfd")
            .level(Level::Error)
            .file(Some("/src/rfd/lib.rs"))
            .args(elsewhere)
            .build();
        assert_eq!(effective_level(&record), Level::Error);
        let ours = format_args!("Invalid FORMATETC structure (0x80040064)");
        let record = Record::builder()
            .target("mini_todo::windows::fence")
            .level(Level::Error)
            .file(Some("crates/app/src/windows/fence/mod.rs"))
            .args(ours)
            .build();
        assert_eq!(effective_level(&record), Level::Error);
    }

    #[test]
    fn third_party_logs_start_at_warn() {
        assert_eq!(level_for("mini_todo::state"), Level::Info);
        assert_eq!(level_for("mini_todo_core::sync"), Level::Info);
        assert_eq!(level_for("wgpu_hal::vulkan"), Level::Warn);
    }
}
