use std::collections::HashMap;
use tokio::sync::{RwLock, Semaphore};

struct ProjectLock {
    running_count: usize,
    max_concurrent: usize,
}

pub struct ConcurrencyManager {
    global_semaphore: Semaphore,
    project_locks: RwLock<HashMap<String, ProjectLock>>,
}

impl ConcurrencyManager {
    pub fn new(max_global: usize) -> Self {
        Self {
            global_semaphore: Semaphore::new(max_global),
            project_locks: RwLock::new(HashMap::new()),
        }
    }

    /// 检查是否可以在指定项目路径上执行任务（非阻塞）
    pub async fn try_acquire(&self, project_path: &str) -> bool {
        if self.global_semaphore.available_permits() == 0 {
            return false;
        }

        let locks = self.project_locks.read().await;
        if let Some(lock) = locks.get(project_path) {
            if lock.running_count >= lock.max_concurrent {
                return false;
            }
        }

        true
    }

    /// 标记项目有新任务开始执行
    pub async fn mark_running(&self, project_path: &str) {
        let mut locks = self.project_locks.write().await;
        let lock = locks
            .entry(project_path.to_string())
            .or_insert(ProjectLock {
                running_count: 0,
                max_concurrent: 1,
            });
        lock.running_count += 1;
    }

    /// 标记项目任务执行结束
    pub async fn mark_finished(&self, project_path: &str) {
        let mut locks = self.project_locks.write().await;
        if let Some(lock) = locks.get_mut(project_path) {
            lock.running_count = lock.running_count.saturating_sub(1);
            if lock.running_count == 0 {
                locks.remove(project_path);
            }
        }
    }

}
