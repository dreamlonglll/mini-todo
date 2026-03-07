use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use tokio::sync::Mutex;

/// 跟踪各项目的最后已知 Git commit hash
pub struct TriggerManager {
    git_hashes: Mutex<HashMap<String, String>>,
    file_watchers: Mutex<HashMap<i64, FileWatchState>>,
}

struct FileWatchState {
    project_path: String,
    last_snapshot: HashMap<String, u64>,
}

impl TriggerManager {
    pub fn new() -> Self {
        Self {
            git_hashes: Mutex::new(HashMap::new()),
            file_watchers: Mutex::new(HashMap::new()),
        }
    }

    // ========== Git Push 触发 ==========

    /// 检查指定项目路径是否有新的 Git commit
    pub async fn check_git_changes(&self, project_path: &str) -> bool {
        let current_hash = match get_git_head(project_path) {
            Some(hash) => hash,
            None => return false,
        };

        let mut hashes = self.git_hashes.lock().await;

        match hashes.get(project_path) {
            Some(last_hash) if last_hash == &current_hash => false,
            Some(_) => {
                hashes.insert(project_path.to_string(), current_hash);
                true
            }
            None => {
                hashes.insert(project_path.to_string(), current_hash);
                false
            }
        }
    }

    /// 初始化项目的 Git hash（不触发）
    pub async fn init_project(&self, project_path: &str) {
        if let Some(hash) = get_git_head(project_path) {
            let mut hashes = self.git_hashes.lock().await;
            hashes.insert(project_path.to_string(), hash);
        }
    }

    /// 获取指定项目最后一次的 commit 信息
    pub async fn get_last_commit_info(&self, project_path: &str) -> Option<String> {
        get_git_last_commit_message(project_path)
    }

    // ========== 文件变更触发 ==========

    /// 注册文件监视（基于轮询快照方式）
    pub async fn register_file_watch(&self, todo_id: i64, project_path: &str) {
        let path_owned = project_path.to_string();
        let snapshot = tokio::task::spawn_blocking(move || {
            scan_directory(&path_owned)
        })
        .await
        .unwrap_or_default();

        let mut watchers = self.file_watchers.lock().await;
        watchers.insert(
            todo_id,
            FileWatchState {
                project_path: project_path.to_string(),
                last_snapshot: snapshot,
            },
        );
    }

    /// 取消文件监视
    pub async fn unregister_file_watch(&self, todo_id: i64) {
        let mut watchers = self.file_watchers.lock().await;
        watchers.remove(&todo_id);
    }

    /// 检查指定 todo 的文件是否有变更
    pub async fn check_file_changes(&self, todo_id: i64) -> bool {
        let project_path = {
            let watchers = self.file_watchers.lock().await;
            match watchers.get(&todo_id) {
                Some(s) => s.project_path.clone(),
                None => return false,
            }
        };

        let new_snapshot = tokio::task::spawn_blocking(move || {
            scan_directory(&project_path)
        })
        .await
        .unwrap_or_default();

        let mut watchers = self.file_watchers.lock().await;
        let state = match watchers.get_mut(&todo_id) {
            Some(s) => s,
            None => return false,
        };

        if new_snapshot != state.last_snapshot {
            state.last_snapshot = new_snapshot;
            true
        } else {
            false
        }
    }

    /// 获取所有注册了文件监视的 todo_id 列表
    pub async fn get_watched_todos(&self) -> Vec<i64> {
        let watchers = self.file_watchers.lock().await;
        watchers.keys().cloned().collect()
    }
}

/// 扫描目录获取文件修改时间快照（排除 .git、node_modules、target）
fn scan_directory(project_path: &str) -> HashMap<String, u64> {
    let mut snapshot = HashMap::new();
    let path = Path::new(project_path);

    if !path.is_dir() {
        return snapshot;
    }

    scan_dir_recursive(path, path, &mut snapshot, 0);
    snapshot
}

fn scan_dir_recursive(
    base: &Path,
    current: &Path,
    snapshot: &mut HashMap<String, u64>,
    depth: u32,
) {
    if depth > 5 {
        return;
    }

    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" || name == "build" {
            continue;
        }

        if path.is_dir() {
            scan_dir_recursive(base, &path, snapshot, depth + 1);
        } else if let Ok(metadata) = std::fs::metadata(&path) {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            snapshot.insert(relative, modified);
        }
    }
}

fn get_git_head(project_path: &str) -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(project_path)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}

fn get_git_last_commit_message(project_path: &str) -> Option<String> {
    Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(project_path)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}
