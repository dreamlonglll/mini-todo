use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Worktree {
    path: PathBuf,
    branch_name: String,
    project_path: PathBuf,
}

impl Worktree {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn branch_name(&self) -> &str {
        &self.branch_name
    }
}

pub struct WorktreeManager;

impl WorktreeManager {
    pub fn create(project_path: &str, task_id: &str) -> Result<Worktree, String> {
        let project = Path::new(project_path);
        let branch_name = format!("agent/{}", task_id);
        let worktree_dir = project.join(".agent-worktrees").join(task_id);

        if let Some(parent) = worktree_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建 worktree 目录失败: {}", e))?;
        }

        let output = Command::new("git")
            .current_dir(project)
            .args(["worktree", "add"])
            .arg(&worktree_dir)
            .args(["-b", &branch_name, "HEAD"])
            .output()
            .map_err(|e| format!("创建 worktree 失败: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "git worktree add 失败: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(Worktree {
            path: worktree_dir,
            branch_name,
            project_path: project.to_path_buf(),
        })
    }

    pub fn collect_diff(worktree: &Worktree) -> Result<String, String> {
        let _ = Command::new("git")
            .current_dir(&worktree.path)
            .args(["add", "-A"])
            .output();

        let output = Command::new("git")
            .current_dir(&worktree.path)
            .args(["diff", "--cached"])
            .output()
            .map_err(|e| format!("获取 diff 失败: {}", e))?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub fn changed_files(worktree: &Worktree) -> Result<Vec<String>, String> {
        let _ = Command::new("git")
            .current_dir(&worktree.path)
            .args(["add", "-A"])
            .output();

        let output = Command::new("git")
            .current_dir(&worktree.path)
            .args(["diff", "--cached", "--name-only"])
            .output()
            .map_err(|e| format!("获取变更文件列表失败: {}", e))?;

        let files = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();

        Ok(files)
    }

    pub fn cleanup(worktree: &Worktree) -> Result<(), String> {
        let _ = Command::new("git")
            .current_dir(&worktree.project_path)
            .args(["worktree", "remove", "--force"])
            .arg(&worktree.path)
            .output();

        let _ = Command::new("git")
            .current_dir(&worktree.project_path)
            .args(["branch", "-D", &worktree.branch_name])
            .output();

        if worktree.path.exists() {
            let _ = std::fs::remove_dir_all(&worktree.path);
        }

        Ok(())
    }

    pub fn merge_to_current(worktree: &Worktree) -> Result<String, String> {
        let _ = Command::new("git")
            .current_dir(&worktree.path)
            .args(["add", "-A"])
            .output();

        let _ = Command::new("git")
            .current_dir(&worktree.path)
            .args(["commit", "-m", &format!("[agent] task completed")])
            .output();

        let merge_output = Command::new("git")
            .current_dir(&worktree.project_path)
            .args([
                "merge",
                &worktree.branch_name,
                "--no-ff",
                "-m",
                &format!("[agent] merge {}", worktree.branch_name),
            ])
            .output()
            .map_err(|e| format!("合并失败: {}", e))?;

        if !merge_output.status.success() {
            return Err(format!(
                "合并冲突: {}",
                String::from_utf8_lossy(&merge_output.stderr)
            ));
        }

        let hash = Command::new("git")
            .current_dir(&worktree.project_path)
            .args(["rev-parse", "HEAD"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        Ok(hash)
    }
}
