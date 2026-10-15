//! 监听收纳根目录（`notify`，Windows 上是 ReadDirectoryChangesW），去抖后回调受影响的收纳文件夹名。

use super::location::{fence_root_dir, normalize_for_compare};
use super::naming::is_valid_folder_name;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// 默认去抖间隔：最后一个事件之后安静这么久才回调
pub const WATCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// 持续有变化时（例如大批量复制）最长多久必须回调一次
const MAX_BATCH_DELAY: Duration = Duration::from_millis(1500);

/// 一次去抖后的变化通知
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FenceChange {
    /// 受影响的收纳文件夹名（去重、排序）：文件夹内有增删改名，或文件夹本身被新建 / 删除 / 改名
    /// （改名时新旧两个名字都会出现）。可能包含不对应任何收纳箱的名字，调用方按数据库匹配后忽略即可
    pub folders: Vec<String>,
    /// 监听出错或事件溢出：调用方应对所有收纳箱重新列举对账
    pub rescan: bool,
}

/// 监听句柄：drop 即停止监听（去抖线程随之退出，不再回调）
pub struct FenceWatcher {
    _watcher: RecommendedWatcher,
    root: PathBuf,
}

impl FenceWatcher {
    /// 正在监听的收纳根目录
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// 去掉 `root` 前缀（先精确匹配，失败再按不区分大小写的规范形式匹配）
fn relative_to(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(rel.to_path_buf());
    }
    let path_n = normalize_for_compare(path);
    let root_n = normalize_for_compare(root);
    let rest = path_n.strip_prefix(&root_n)?.strip_prefix('/')?;
    // 规范形式是小写的，按段数从原路径尾部取回原始大小写
    let depth = rest.split('/').filter(|s| !s.is_empty()).count();
    let comps: Vec<_> = path.components().collect();
    let start = comps.len().checked_sub(depth)?;
    Some(comps[start..].iter().collect())
}

/// 事件路径 → 收纳文件夹名。只关心根目录下第一、二层（收纳文件夹本身与其中的项目），
/// 更深层（收纳的文件夹内部）的变化不影响收纳箱显示，忽略。
pub(crate) fn folder_of(root: &Path, path: &Path) -> Option<String> {
    let rel = relative_to(path, root)?;
    let mut comps = rel.components();
    let Some(Component::Normal(first)) = comps.next() else {
        return None;
    };
    if comps.count() > 1 {
        return None;
    }
    let name = first.to_str()?;
    is_valid_folder_name(name).then(|| name.to_string())
}

/// 去抖循环：收到事件后等待 `debounce` 无新事件（或距第一个事件超过 `MAX_BATCH_DELAY`）再回调
fn debounce_loop(
    rx: mpsc::Receiver<notify::Result<Event>>,
    root: PathBuf,
    debounce: Duration,
    on_change: Box<dyn Fn(FenceChange) + Send>,
) {
    let mut folders: BTreeSet<String> = BTreeSet::new();
    let mut rescan = false;
    let mut first_at: Option<Instant> = None;
    loop {
        let received = match first_at {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(first) => {
                let remaining = MAX_BATCH_DELAY.saturating_sub(first.elapsed());
                rx.recv_timeout(debounce.min(remaining))
            }
        };
        let mut flush = false;
        match received {
            Ok(Ok(event)) => {
                if matches!(event.kind, EventKind::Access(_)) {
                    continue;
                }
                let before = (folders.len(), rescan);
                if event.need_rescan() {
                    rescan = true;
                }
                folders.extend(event.paths.iter().filter_map(|p| folder_of(&root, p)));
                if (folders.len(), rescan) != before && first_at.is_none() {
                    first_at = Some(Instant::now());
                }
            }
            Ok(Err(e)) => {
                log::warn!("[fence] 监听收纳目录出错: {}", e);
                rescan = true;
                first_at.get_or_insert_with(Instant::now);
            }
            Err(RecvTimeoutError::Timeout) => flush = true,
            // 监听已停止（FenceWatcher 被 drop）：不再回调
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if first_at.is_some_and(|first| first.elapsed() >= MAX_BATCH_DELAY) {
            flush = true;
        }
        if flush && first_at.is_some() {
            let change = FenceChange {
                folders: std::mem::take(&mut folders).into_iter().collect(),
                rescan: std::mem::take(&mut rescan),
            };
            first_at = None;
            // 回调 panic 不能终止监听
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| on_change(change)));
        }
    }
}

pub(crate) fn watch_root_at(
    root: PathBuf,
    debounce: Duration,
    on_change: impl Fn(FenceChange) + Send + 'static,
) -> Result<FenceWatcher, String> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher =
        notify::recommended_watcher(tx).map_err(|e| format!("创建目录监听失败: {}", e))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("监听收纳目录失败（{}）: {}", root.display(), e))?;
    let loop_root = root.clone();
    let callback: Box<dyn Fn(FenceChange) + Send> = Box::new(on_change);
    std::thread::Builder::new()
        .name("fence-watch".to_string())
        .spawn(move || debounce_loop(rx, loop_root, debounce, callback))
        .map_err(|e| format!("创建监听线程失败: {}", e))?;
    Ok(FenceWatcher {
        _watcher: watcher,
        root,
    })
}

/// 监听收纳根目录（不存在则创建），变化经 300ms 去抖后在内部线程 `fence-watch` 上回调。
///
/// * 回调在后台线程执行，里面可以直接 `app_handle.emit_to(...)` 通知对应收纳箱窗口重新列举
/// * 返回的 [`FenceWatcher`] 需要一直持有（例如放进 Tauri 托管状态），drop 即停止监听
/// * 本应用自己的移入 / 移回操作同样会触发回调，调用方据此统一刷新即可，不需要区分来源
pub fn watch_fence_root(
    on_change: impl Fn(FenceChange) + Send + 'static,
) -> Result<FenceWatcher, String> {
    watch_root_at(fence_root_dir()?, WATCH_DEBOUNCE, on_change)
}

#[cfg(test)]
mod tests {
    use super::super::folder::test_support::{touch, TempDir};
    use super::*;

    #[test]
    fn watcher_can_live_in_tauri_state() {
        // Tauri `manage` 要求 Send + Sync：Mutex<FenceWatcher> 需要 FenceWatcher: Send
        fn assert_send<T: Send + 'static>() {}
        assert_send::<FenceWatcher>();
    }

    #[test]
    fn folder_of_maps_paths_to_fence_folders() {
        let root = Path::new("/r/MiniTodo收纳箱");
        assert_eq!(
            folder_of(root, &root.join("常用").join("a.lnk")).as_deref(),
            Some("常用")
        );
        assert_eq!(folder_of(root, &root.join("常用")).as_deref(), Some("常用"));
        // 收纳的文件夹内部：忽略
        assert_eq!(
            folder_of(root, &root.join("常用").join("sub").join("x")),
            None
        );
        // 根目录本身 / 根目录之外
        assert_eq!(folder_of(root, root), None);
        assert_eq!(folder_of(root, Path::new("/r/other/a")), None);
        // 名字不合法的（不可能是收纳文件夹）
        assert_eq!(folder_of(root, &root.join("CON")), None);
    }

    #[cfg(windows)]
    #[test]
    fn folder_of_is_case_insensitive_on_windows() {
        let root = Path::new(r"C:\Users\me\MiniTodo收纳箱");
        assert_eq!(
            folder_of(root, Path::new(r"c:\users\ME\minitodo收纳箱\Tools\a.lnk")).as_deref(),
            Some("Tools")
        );
    }

    #[test]
    fn watcher_reports_debounced_folder_changes() {
        let tmp = TempDir::new("watch");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("甲")).unwrap();
        std::fs::create_dir_all(root.join("乙")).unwrap();
        let (tx, rx) = mpsc::channel::<FenceChange>();
        let watcher = watch_root_at(root.clone(), Duration::from_millis(150), move |change| {
            let _ = tx.send(change);
        })
        .unwrap();
        assert_eq!(watcher.root(), root.as_path());

        // 同一时间段内的多次变化合并成一次回调
        touch(&root.join("甲").join("a.lnk"));
        touch(&root.join("甲").join("b.lnk"));
        touch(&root.join("乙").join("c.lnk"));
        touch(&root.join("乙").join("sub").join("deep.txt"));
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !(seen.contains("甲") && seen.contains("乙")) && Instant::now() < deadline {
            if let Ok(change) = rx.recv_timeout(Duration::from_secs(1)) {
                seen.extend(change.folders);
            }
        }
        assert!(seen.contains("甲") && seen.contains("乙"), "{seen:?}");

        // 收纳文件夹改名：新旧名字都报告
        while rx.recv_timeout(Duration::from_millis(400)).is_ok() {}
        std::fs::rename(root.join("甲"), root.join("丙")).unwrap();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !(seen.contains("甲") && seen.contains("丙")) && Instant::now() < deadline {
            if let Ok(change) = rx.recv_timeout(Duration::from_secs(1)) {
                seen.extend(change.folders);
            }
        }
        assert!(seen.contains("甲") && seen.contains("丙"), "{seen:?}");

        // 停止监听后不再回调
        drop(watcher);
        while rx.recv_timeout(Duration::from_millis(400)).is_ok() {}
        touch(&root.join("乙").join("after-drop.lnk"));
        assert!(rx.recv_timeout(Duration::from_millis(800)).is_err());
    }
}
